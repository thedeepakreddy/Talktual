// Phase 8 — ambient translation: listen to the room (intentionally, visibly),
// recognise the chosen language, translate, and show and/or speak the result.
//
// Built on the existing engine, not beside it:
//   * recognition      → AsrFactory port (Web Speech, continuous mode, auto-restart)
//   * translation      → Translator port via privacy/routing.ts (Phase 6 policy, shared with calls)
//   * speech output    → PlaybackPort (bounded FIFO, stale drop, cancel, device/cloud route)
//   * processing modes → ProcessingPolicy + RuntimeRegistry + NetworkMonitor
//   * diagnostics      → Diagnostics (allowlisted enums/numbers only — never text)
//
// Lifecycle (explicit, never automatic):
//   idle ─start→ (consent) → starting → listening ⇄ paused → stopped
//                                   ↘ unavailable / stalled (visible, with a way out)
// Guarantees:
//   * The microphone/recognizer runs only in `listening` with capture active.
//     Pause/stop/hidden/offline abort it immediately (abort = in-flight audio discarded).
//   * Nothing captured is persisted. Lines live in memory (bounded) until Clear or dispose.
//   * Ordering: speech is enqueued strictly in commit order; a phrase whose
//     translation is late or failed is skipped, never played out of order or twice.
//   * Bounded: lines, pending translations, in-flight translations, restarts.
//   * Half-duplex unless the user says they listen through headphones: capture
//     is suspended while translated speech plays (prevents a feedback loop), and
//     recognised text matching what was just spoken is dropped (echo guard).

import type { AsrAdapter, AsrErrorCode, RecognizedSegment, TranslationResult, TtsRoute } from '../engine/ports';
import type { SessionDeps } from '../engine/CommunicationSession';
import type { Disclosure } from '../engine/privacy';
import { Diagnostics } from '../engine/diagnostics';
import { PhrasebookRuntime } from '../local/phrasebook';
import { RuntimeRegistry } from '../local/registry';
import { browserNetworkSource, NetworkMonitor, type NetworkSource } from '../privacy/network';
import { PolicyBlockedError, PROCESSING_MODES, ProcessingPolicy, type NetworkState, type ProcessingMode } from '../privacy/policy';
import { routedTranslate } from '../privacy/routing';
import { ambientConsentKey, ambientDisclosure } from './disclosure';
import { LanguageMismatchDetector } from './langGuess';
import { StablePrefixSegmenter, type AmbientUnit } from './stablePrefix';

export type AmbientPhase = 'idle' | 'starting' | 'listening' | 'paused' | 'stalled' | 'unavailable' | 'stopped';
export type PauseReason = 'user' | 'hidden' | 'offline' | 'mic' | 'policy';
export type CapState = 'device' | 'cloud' | 'device+cloud' | 'unavailable';

export interface AmbientLine {
  id: string;
  original: string;
  translation: string | null;
  state: 'translating' | 'translated' | 'untranslated' | 'skipped';
  /** Why it isn't translated/spoken, in plain words (shown quietly). */
  note?: string;
  speech: 'off' | 'waiting' | 'queued' | 'playing' | 'played' | 'skipped' | 'failed';
  early: boolean;
  revised: boolean;
  uncertain: boolean;
  heardAt: number;
  location?: 'device' | 'cloud' | 'none';
}

export interface AmbientNotice {
  code: string;
  message: string;
  tone: 'info' | 'warning' | 'error';
}

export interface AmbientSnapshot {
  phase: AmbientPhase;
  pauseReason: PauseReason | null;
  /** The recognizer is actually running right now (false while speech plays half-duplex). */
  capturing: boolean;
  /** Capture is held back because translated speech is playing (no headphones). */
  heldForSpeech: boolean;
  hearing: boolean;
  from: string;
  to: string;
  speak: boolean;
  headphones: boolean;
  volume: number;
  lines: AmbientLine[];
  partial: string;
  notice: AmbientNotice | null;
  /** The room may be speaking another language (base code), never acted on automatically. */
  languageHint: string | null;
  consent: { prompt: boolean; disclosure: Disclosure | null; granted: boolean };
  processing: { available: boolean; mode: ProcessingMode; network: NetworkState; asr: CapState; translate: CapState; speech: CapState };
  playingId: string | null;
}

export interface AmbientConfig {
  from: string;
  to: string;
  speak?: boolean;
  headphones?: boolean;
  volume?: number;
}

export interface VisibilitySource {
  hidden(): boolean;
  subscribe(fn: () => void): () => void;
}

export interface AudioSessionLike {
  type: string;
}

export interface AmbientDeps extends Pick<SessionDeps, 'createAsr' | 'translate' | 'playback' | 'capture' | 'browser' | 'storage' | 'getServerCapabilities' | 'clearCaches' | 'onUserGesture' | 'processing'> {
  now?: () => number;
  setTimeout?: typeof setTimeout;
  clearTimeout?: typeof clearTimeout;
  visibility?: VisibilitySource | null;
  /** WebKit Audio Session API (`navigator.audioSession`) where present. */
  audioSession?: AudioSessionLike | null;
}

export interface AmbientLimits {
  maxLines: number;
  maxPending: number;
  maxInFlight: number;
  /** A translation not ready to speak within this long after it was heard is shown but not spoken. */
  maxSpeechLagMs: number;
  /** No recognizer activity for this long while listening → restart the recognizer. */
  watchdogMs: number;
  maxRestartsPerMinute: number;
  /** After translated speech ends, wait this long before capture resumes (room echo decay). */
  resumeAfterSpeechMs: number;
  echoWindowMs: number;
}

export const DEFAULT_AMBIENT_LIMITS: AmbientLimits = {
  maxLines: 200,
  maxPending: 8,
  maxInFlight: 2,
  maxSpeechLagMs: 9000,
  watchdogMs: 25000,
  maxRestartsPerMinute: 3,
  resumeAfterSpeechMs: 350,
  echoWindowMs: 12000,
};

const PROCESSING_STORAGE_KEY = 'talktual.processing.mode.v1';
const ID_PREFIX = 'amb:';


function defaultVisibility(): VisibilitySource | null {
  if (typeof document === 'undefined') return null;
  return {
    hidden: () => document.visibilityState === 'hidden',
    subscribe: (fn) => {
      document.addEventListener('visibilitychange', fn);
      return () => document.removeEventListener('visibilitychange', fn);
    },
  };
}

const words = (s: string) => s.toLocaleLowerCase().split(/[^\p{L}\p{N}']+/u).filter(Boolean);
function overlap(a: string, b: string): number {
  const A = new Set(words(a));
  const B = new Set(words(b));
  if (!A.size || !B.size) return 0;
  let n = 0;
  for (const w of A) if (B.has(w)) n++;
  return n / Math.min(A.size, B.size);
}

type Listener = (s: AmbientSnapshot) => void;

export class AmbientSession {
  private snap: AmbientSnapshot;
  private listeners = new Set<Listener>();
  private readonly now: () => number;
  private readonly setT: typeof setTimeout;
  private readonly clearT: typeof clearTimeout;
  private readonly limits: AmbientLimits;
  readonly diag: Diagnostics;

  private asr: AsrAdapter | null = null;
  private asrGate: { signal: AbortSignal; end: () => void } | null = null;
  private seg: StablePrefixSegmenter;
  private langCheck: LanguageMismatchDetector;
  private policy: ProcessingPolicy | null = null;
  private registry: RuntimeRegistry | null = null;
  private netmon: NetworkMonitor | null = null;
  private netSource: NetworkSource | null = null;
  private translatorName: string | null = null;
  private serverTts = false;

  private pending: AmbientUnit[] = [];
  private inFlight = new Map<string, AbortController>();
  private speakCursor = 0; // index into `order` of the next line to consider for speech
  private order: string[] = []; // line ids in commit order (bounded with lines)
  private meta = new Map<string, { heardAt: number; committedAt: number; translatedAt?: number }>();
  private spoken: { t: number; text: string }[] = [];
  private lastActivity = 0;
  private restarts: number[] = [];
  private watchdog: ReturnType<typeof setTimeout> | null = null;
  private resumeTimer: ReturnType<typeof setTimeout> | null = null;
  private savedAudioSessionType: string | null = null;
  private unsubs: (() => void)[] = [];
  private disposed = false;
  private pendingStart = false;
  private idSeq = 0;

  constructor(
    config: AmbientConfig,
    private readonly deps: AmbientDeps,
    limits: Partial<AmbientLimits> = {}
  ) {
    this.now = deps.now ?? (() => Date.now());
    this.setT = deps.setTimeout ?? setTimeout;
    this.clearT = deps.clearTimeout ?? clearTimeout;
    this.limits = { ...DEFAULT_AMBIENT_LIMITS, ...limits };
    this.diag = new Diagnostics(300, 200, this.now);
    this.seg = new StablePrefixSegmenter(config.from, (u) => this.onUnit(u), this.now, undefined, () => `u${(++this.idSeq).toString(36)}`);
    this.langCheck = new LanguageMismatchDetector(config.from);

    let mode: ProcessingMode = 'cloud';
    if (deps.processing) {
      const opts = typeof deps.processing === 'object' ? deps.processing : {};
      let saved: string | null = null;
      try {
        saved = deps.storage?.getItem(PROCESSING_STORAGE_KEY) ?? null;
      } catch {
        saved = null;
      }
      mode = opts.mode ?? (PROCESSING_MODES.includes(saved as ProcessingMode) ? (saved as ProcessingMode) : 'cloud');
      this.policy = new ProcessingPolicy(mode, this.now);
      this.registry = new RuntimeRegistry({ now: this.now, setTimeout: (fn, ms) => this.setT(fn, ms), clearTimeout: (t) => this.clearT(t as ReturnType<typeof setTimeout>) }, opts.registryLimits);
      this.registry.addTranslator(new PhrasebookRuntime(this.now));
      for (const t of opts.translators ?? []) this.registry.addTranslator(t);
      for (const v of opts.voices ?? []) this.registry.addVoice(v);
      for (const a of opts.asr ?? []) this.registry.addAsr(a);
      this.netSource = opts.network ?? browserNetworkSource(typeof window !== 'undefined' ? window : undefined);
    }

    this.snap = {
      phase: 'idle',
      pauseReason: null,
      capturing: false,
      heldForSpeech: false,
      hearing: false,
      from: config.from,
      to: config.to,
      speak: config.speak ?? true,
      headphones: config.headphones ?? false,
      volume: clamp01(config.volume ?? 0.8),
      lines: [],
      partial: '',
      notice: null,
      languageHint: null,
      consent: { prompt: false, disclosure: null, granted: this.hasConsent() },
      processing: { available: Boolean(this.policy), mode, network: 'online', asr: 'cloud', translate: 'cloud', speech: 'cloud' },
      playingId: null,
    };

    // Speech playback status for OUR items only (the controller is shared with calls).
    this.unsubs.push(deps.playback.onStatus((id, status) => this.onPlayback(id, status)));

    const vis = deps.visibility === undefined ? defaultVisibility() : deps.visibility;
    if (vis) {
      this.unsubs.push(
        vis.subscribe(() => {
          if (vis.hidden() && this.snap.phase === 'listening') this.pauseFor('hidden');
        })
      );
    }
    this.unsubs.push(deps.capture.onDeviceLost(() => this.onAsrError('device-lost', 'device lost', true)));

    if (this.policy && this.netSource) {
      const policy = this.policy;
      this.netmon = new NetworkMonitor(this.netSource, this.now, (s) => policy.setNetwork(s));
      this.unsubs.push(policy.onChange(() => this.onPolicyChange()));
      policy.setNetwork(this.netmon.state);
      this.registry!.discover()
        .then(() => this.refreshCapabilities())
        .catch(() => this.refreshCapabilities());
    }
    deps
      .getServerCapabilities()
      .then((c) => {
        this.translatorName = c.translator;
        this.serverTts = c.tts;
        this.refreshCapabilities();
      })
      .catch(() => this.refreshCapabilities());
    this.refreshCapabilities();
  }

  // ===========================================================================
  // Public API
  // ===========================================================================

  getSnapshot(): AmbientSnapshot {
    return this.snap;
  }

  subscribe(fn: Listener): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Explicit activation. Asks for consent first if this device hasn't given it for ambient listening. */
  start(): void {
    if (this.disposed) return;
    this.deps.onUserGesture?.();
    if (this.snap.phase === 'listening' || this.snap.phase === 'starting') return;
    if (!this.hasConsent()) {
      this.pendingStart = true;
      this.commit({ consent: { prompt: true, disclosure: this.disclosure(), granted: false } });
      return;
    }
    this.begin();
  }

  grantConsent(): void {
    try {
      this.deps.storage?.setItem(ambientConsentKey('browser-speech'), 'granted');
    } catch {
      /* lasts for this page */
    }
    this.consentThisPage = true;
    this.commit({ consent: { prompt: false, disclosure: null, granted: true } });
    this.diag.count('ambient_consent_granted');
    if (this.pendingStart) {
      this.pendingStart = false;
      this.begin();
    }
  }

  declineConsent(): void {
    this.pendingStart = false;
    this.commit({ consent: { ...this.snap.consent, prompt: false, disclosure: null } });
  }

  /** Re-show what is captured and where it goes (informational once granted). */
  showDisclosure(): void {
    this.commit({ consent: { ...this.snap.consent, prompt: true, disclosure: this.disclosure() } });
  }

  pause(): void {
    if (this.snap.phase === 'listening' || this.snap.phase === 'stalled' || this.snap.phase === 'starting') this.pauseFor('user');
  }

  resume(): void {
    if (this.disposed) return;
    if (this.snap.phase === 'paused' || this.snap.phase === 'stalled' || this.snap.phase === 'unavailable') {
      this.restarts = [];
      this.begin();
    }
  }

  /** Stop listening and speaking. The text stays on screen until Clear. */
  stop(): void {
    if (this.disposed) return;
    this.cancelOutput();
    this.stopCapture('stop');
    this.restoreAudioSession();
    this.commit({ phase: 'stopped', pauseReason: null, notice: null });
    this.diag.record('ambient', { state: 'stopped' });
  }

  /** Drop everything this session knows: lines, caches, routing audit, loaded runtimes. */
  clear(): void {
    this.cancelOutput();
    this.order = [];
    this.meta.clear();
    this.speakCursor = 0;
    this.spoken = [];
    this.seg.reset();
    this.langCheck.reset();
    this.deps.clearCaches?.();
    this.policy?.clearAudit();
    this.registry?.unloadAll();
    this.commit({ lines: [], partial: '', languageHint: null });
    this.diag.count('ambient_cleared');
  }

  /** Text-only when false. Turning speech off stops what is playing now. */
  setSpeak(on: boolean): void {
    if (on === this.snap.speak) return;
    if (!on) this.cancelOutput(true, true);
    this.commit({ speak: on });
    if (on) this.resumeCaptureIfHeld();
  }

  setHeadphones(on: boolean): void {
    this.commit({ headphones: on });
    if (on) this.resumeCaptureIfHeld();
  }

  setVolume(v: number): void {
    this.commit({ volume: clamp01(v) });
  }

  setLanguages(from: string, to: string): void {
    if (from === this.snap.from && to === this.snap.to) return;
    const wasListening = this.snap.phase === 'listening';
    if (wasListening) this.stopCapture('language');
    this.cancelOutput();
    this.seg = new StablePrefixSegmenter(from, (u) => this.onUnit(u), this.now, undefined, () => `u${(++this.idSeq).toString(36)}`);
    this.langCheck.setExpected(from);
    this.commit({ from, to, languageHint: null, partial: '' });
    this.refreshCapabilities();
    if (wasListening) this.begin();
  }

  dismissLanguageHint(): void {
    this.langCheck.reset();
    this.commit({ languageHint: null });
  }

  /** Speak one line again (a deliberate request: plays even if late). */
  replay(lineId: string): void {
    const line = this.snap.lines.find((l) => l.id === lineId);
    if (!line?.translation || line.state !== 'translated') return;
    this.enqueueSpeech(line, true);
  }

  setProcessingMode(mode: ProcessingMode): void {
    const policy = this.policy;
    if (!policy || !PROCESSING_MODES.includes(mode) || mode === policy.mode) return;
    policy.setMode(mode);
    try {
      this.deps.storage?.setItem(PROCESSING_STORAGE_KEY, mode);
    } catch {
      /* page only */
    }
    this.deps.clearCaches?.();
    this.diag.record('processing', { mode });
  }

  /** Enum/number-only diagnostics (stage latencies, counters). Never text. */
  diagnostics() {
    return { ...this.diag.snapshot(), segmenter: { ...this.seg.stats }, policy: this.policy ? { ...this.policy.counts } : null };
  }

  dispose(): void {
    if (this.disposed) return;
    this.cancelOutput();
    this.stopCapture('dispose');
    this.restoreAudioSession();
    this.disposed = true;
    if (this.watchdog) this.clearT(this.watchdog);
    if (this.resumeTimer) this.clearT(this.resumeTimer);
    this.unsubs.forEach((u) => u());
    this.unsubs = [];
    this.netmon?.dispose();
    this.registry?.dispose();
    this.deps.clearCaches?.();
    this.listeners.clear();
    // Transcript is dropped with the session object (never persisted).
    this.snap = { ...this.snap, lines: [], partial: '', phase: 'stopped', capturing: false };
  }

  // ===========================================================================
  // Capture
  // ===========================================================================

  private consentThisPage = false;

  private hasConsent(): boolean {
    if (this.consentThisPage) return true;
    try {
      return this.deps.storage?.getItem(ambientConsentKey('browser-speech')) === 'granted';
    } catch {
      return false;
    }
  }

  private disclosure(): Disclosure {
    return ambientDisclosure(this.translatorName, this.policy?.mode ?? 'cloud');
  }

  private begin(): void {
    if (this.disposed) return;
    const b = this.deps.browser;
    if (!b.secureContext) return this.unavailable('insecure', 'Live translate needs a secure (https) connection.');
    if (!b.webSpeech) return this.unavailable('no-speech-recognition', 'This browser can’t recognise speech. Use Chrome, Edge or Safari for live translate.');
    if (this.policy?.network === 'offline') {
      this.commit({ phase: 'paused', pauseReason: 'offline', notice: { code: 'offline', message: 'You’re offline. Listening resumes when you’re back online.', tone: 'warning' } });
      return;
    }
    this.commit({ phase: 'starting', pauseReason: null, notice: null, languageHint: null });
    this.applyAudioSession();
    this.startCapture();
  }

  private startCapture(): void {
    if (this.asr || this.disposed) return;
    let opts: { processLocally?: boolean } | undefined;
    if (this.policy) {
      const plan = this.policy.plan('asr', { deviceAvailable: Boolean(this.registry?.hasLocalAsr(this.snap.from)), cloudAvailable: this.deps.browser.webSpeech, cloudConsent: this.hasConsent() });
      if (!plan.steps.length) {
        const msg =
          plan.reason === 'local-only'
            ? 'On-device mode: this device can’t recognise this language on its own. Switch to Hybrid or Cloud to listen.'
            : plan.reason === 'offline'
              ? 'You’re offline, and this device can’t recognise speech on its own.'
              : 'Speech recognition isn’t available right now.';
        this.unavailable(`asr-${plan.reason}`, msg);
        return;
      }
      if (plan.steps[0] === 'device') {
        opts = { processLocally: true };
        this.policy.noteDevice('asr', plan.reason);
      } else {
        try {
          const gate = this.policy.beginRemote('asr', { cloudConsent: this.hasConsent() });
          this.asrGate = gate;
          gate.signal.addEventListener('abort', () => {
            if (this.asrGate === gate) this.pauseFor(this.policy?.network === 'offline' ? 'offline' : 'policy');
          });
        } catch (e) {
          if (e instanceof PolicyBlockedError) return this.unavailable('asr-blocked', 'Your processing settings don’t allow cloud speech recognition.');
          throw e;
        }
      }
      this.diag.record('route', { kind: 'asr', status: plan.steps[0], reason: plan.reason });
    }
    let created: AsrAdapter | null = null;
    const current = () => created !== null && this.asr === created;
    created = this.deps.createAsr(
      this.snap.from,
      'continuous',
      {
        onPartial: (text) => {
          if (!current()) return;
          this.lastActivity = this.now();
          this.seg.partial(text);
          this.commit({ partial: text });
        },
        onFinal: (s: RecognizedSegment) => {
          if (!current()) return this.diag.count('stale_asr_results_dropped');
          this.lastActivity = this.now();
          this.seg.final(s.text, s.confidence);
          this.commit({ partial: '' });
        },
        onSpeechActivity: (active) => {
          if (!current()) return;
          this.lastActivity = this.now();
          this.commit({ hearing: active });
        },
        onError: (code, message, fatal) => {
          if (current()) this.onAsrError(code, message, fatal);
        },
        onEnd: () => {
          if (current()) this.onAsrEnded();
        },
      },
      opts
    );
    this.asr = created;
    this.lastActivity = this.now();
    this.armWatchdog();
    const asr = created;
    asr
      .start()
      .then(() => {
        if (this.asr !== asr || !asr.active) return;
        this.commit({ phase: 'listening', capturing: true, heldForSpeech: false });
        this.diag.record('ambient', { state: 'listening' });
      })
      .catch(() => {
        /* reported through onError */
      });
  }

  /** Abort (discard in-flight audio) and release the microphone. */
  private stopCapture(reason: string): void {
    const asr = this.asr;
    this.asr = null;
    this.asrGate?.end();
    this.asrGate = null;
    if (this.watchdog) this.clearT(this.watchdog);
    this.watchdog = null;
    if (this.resumeTimer) this.clearT(this.resumeTimer);
    this.resumeTimer = null;
    if (asr) {
      asr.abort();
      asr.dispose();
      this.deps.capture.release();
      this.diag.record('asr_stop', { reason });
    }
    this.seg.reset();
    this.commit({ capturing: false, hearing: false, partial: '' });
  }

  private pauseFor(reason: PauseReason, notice?: AmbientNotice): void {
    this.stopCapture(`pause-${reason}`);
    this.restoreAudioSession();
    const messages: Partial<Record<PauseReason, AmbientNotice>> = {
      hidden: { code: 'hidden', message: 'Paused because Talktual left the screen. Resume when you’re ready.', tone: 'info' },
      offline: { code: 'offline', message: 'You’re offline. Listening resumes when you’re back online.', tone: 'warning' },
      policy: { code: 'policy', message: 'Paused: your processing settings no longer allow this. Change them to continue.', tone: 'warning' },
    };
    this.commit({ phase: 'paused', pauseReason: reason, heldForSpeech: false, notice: notice ?? messages[reason] ?? null });
    this.diag.record('ambient', { state: 'paused', reason });
  }

  private unavailable(code: string, message: string): void {
    this.stopCapture(code);
    this.restoreAudioSession();
    this.commit({ phase: 'unavailable', notice: { code, message, tone: 'error' } });
    this.diag.record('ambient', { state: 'unavailable', reason: code });
  }

  private onAsrError(code: AsrErrorCode, _message: string, fatal: boolean): void {
    this.diag.record('asr_error', { code, fatal });
    if (code === 'permission-denied') {
      this.unavailable('permission-denied', 'Microphone access is blocked. Allow it in your browser’s site settings, then press Start again.');
      return;
    }
    if (code === 'device-lost' || code === 'capture-unavailable') {
      if (this.snap.phase !== 'listening' && this.snap.phase !== 'starting') return;
      this.pauseFor('mic', { code, message: code === 'device-lost' ? 'The microphone was disconnected. Reconnect it and press Resume.' : 'No microphone is available.', tone: 'error' });
      return;
    }
    if (fatal) this.restartRecognizer('fatal-error');
  }

  /** The recognizer ended without being asked (the adapter already restarts routine ends). */
  private onAsrEnded(): void {
    if (this.snap.phase === 'listening' || this.snap.phase === 'starting') this.restartRecognizer('ended');
  }

  /** Bounded restart: at most N per minute, then a visible stalled state. */
  private restartRecognizer(reason: string): void {
    const t = this.now();
    this.restarts = this.restarts.filter((r) => t - r < 60_000);
    this.stopCapture(`restart-${reason}`);
    if (this.restarts.length >= this.limits.maxRestartsPerMinute) {
      this.restoreAudioSession();
      this.commit({ phase: 'stalled', notice: { code: 'stalled', message: 'Listening stopped responding. Press Resume to try again.', tone: 'error' } });
      this.diag.count('ambient_stalled');
      return;
    }
    this.restarts.push(t);
    this.diag.count('asr_restarts');
    this.startCapture();
  }

  private armWatchdog(): void {
    if (this.watchdog) this.clearT(this.watchdog);
    this.watchdog = this.setT(() => {
      this.watchdog = null;
      if (!this.asr) return;
      if (this.now() - this.lastActivity >= this.limits.watchdogMs) this.restartRecognizer('silent');
      else this.armWatchdog();
    }, Math.min(5000, this.limits.watchdogMs));
  }

  private applyAudioSession(): void {
    const s = this.deps.audioSession === undefined ? defaultAudioSession() : this.deps.audioSession;
    if (!s || this.savedAudioSessionType !== null) return;
    try {
      this.savedAudioSessionType = s.type;
      s.type = 'play-and-record';
    } catch {
      this.savedAudioSessionType = null;
    }
  }

  private restoreAudioSession(): void {
    const s = this.deps.audioSession === undefined ? defaultAudioSession() : this.deps.audioSession;
    if (!s || this.savedAudioSessionType === null) return;
    try {
      s.type = this.savedAudioSessionType;
    } catch {
      /* best effort */
    }
    this.savedAudioSessionType = null;
  }

  // ===========================================================================
  // Units → lines → translation → speech
  // ===========================================================================

  private onUnit(u: AmbientUnit): void {
    // Echo guard: our own translated speech picked up by the microphone.
    if (!this.snap.headphones) {
      const t = this.now();
      this.spoken = this.spoken.filter((s) => t - s.t <= this.limits.echoWindowMs);
      if (this.spoken.some((s) => overlap(s.text, u.text) >= 0.6)) {
        this.diag.count('echo_dropped');
        return;
      }
    }
    const hint = this.langCheck.add(u.text);
    if (hint && hint !== this.snap.languageHint) this.commit({ languageHint: hint });
    const id = ID_PREFIX + u.id;
    const line: AmbientLine = {
      id,
      original: u.text,
      translation: null,
      state: 'translating',
      speech: this.snap.speak ? 'waiting' : 'off',
      early: u.kind === 'early',
      revised: u.revised,
      uncertain: u.uncertain,
      heardAt: u.heardAt,
    };
    this.meta.set(id, { heardAt: u.heardAt, committedAt: u.committedAt });
    this.diag.sample('amb_commit_ms', u.committedAt - u.heardAt);
    this.order.push(id);
    this.appendLine(line);
    this.pending.push({ ...u, id });
    // Backlog bound: if the room talks faster than we can translate, skip the oldest waiting phrase.
    while (this.pending.length > this.limits.maxPending) {
      const dropped = this.pending.shift()!;
      this.updateLine(dropped.id, { state: 'skipped', note: 'Skipped to keep up', speech: 'skipped' });
      this.diag.count('backlog_skipped');
    }
    this.pump();
    this.flushSpeech();
  }

  private appendLine(line: AmbientLine): void {
    let lines = [...this.snap.lines, line];
    if (lines.length > this.limits.maxLines) {
      const drop = lines.length - this.limits.maxLines;
      const gone = new Set(lines.slice(0, drop).map((l) => l.id));
      lines = lines.slice(drop);
      for (const id of gone) {
        this.meta.delete(id);
        this.inFlight.get(id)?.abort();
      }
      const before = this.order.length;
      this.order = this.order.filter((id) => !gone.has(id));
      this.speakCursor = Math.max(0, this.speakCursor - (before - this.order.length));
    }
    this.commit({ lines });
  }

  private updateLine(id: string, patch: Partial<AmbientLine>): void {
    let changed = false;
    const lines = this.snap.lines.map((l) => {
      if (l.id !== id) return l;
      changed = true;
      return { ...l, ...patch };
    });
    if (changed) this.commit({ lines });
  }

  private pump(): void {
    while (this.inFlight.size < this.limits.maxInFlight && this.pending.length) {
      const u = this.pending.shift()!;
      void this.translateUnit(u);
    }
  }

  private async translateUnit(u: AmbientUnit): Promise<void> {
    const ctrl = new AbortController();
    this.inFlight.set(u.id, ctrl);
    const t0 = this.now();
    const { from, to } = this.snap;
    let result: TranslationResult;
    let location: AmbientLine['location'] = 'none';
    try {
      if (from.split('-')[0] === to.split('-')[0]) {
        result = { segmentId: u.id, status: 'passthrough', text: u.text, attempts: 0, latencyMs: 0 };
        location = 'device';
      } else if (this.policy && this.registry) {
        const r = await routedTranslate(this.policy, this.registry, this.deps.translate, { segmentId: u.id, text: u.text, from, to, signal: ctrl.signal }, {}, {
          record: (type, fields) => this.diag.record(type, fields),
          count: (name) => this.diag.count(name),
          networkFailure: () => this.netmon?.networkFailure(),
          networkSuccess: () => this.netmon?.success(),
        });
        result = r.result;
        location = r.location;
      } else {
        result = await this.deps.translate({ segmentId: u.id, text: u.text, from, to, signal: ctrl.signal });
        location = result.status === 'translated' ? 'cloud' : 'none';
      }
    } catch {
      result = { segmentId: u.id, status: 'failed', text: u.text, error: { code: 'unavailable', message: 'error' }, attempts: 0, latencyMs: 0 };
    } finally {
      this.inFlight.delete(u.id);
    }
    if (this.disposed) return;
    const m = this.meta.get(u.id);
    if (!m) return this.pump(); // line was cleared/dropped meanwhile
    if (result.status === 'cancelled') {
      this.updateLine(u.id, { state: 'skipped', note: 'Cancelled', speech: 'skipped' });
    } else if (result.status === 'translated' || result.status === 'passthrough') {
      m.translatedAt = this.now();
      this.diag.sample('amb_translate_ms', m.translatedAt - t0);
      this.diag.sample('amb_text_ms', m.translatedAt - m.heardAt);
      this.updateLine(u.id, { state: 'translated', translation: result.text, location });
    } else {
      const why = result.error?.code === 'unavailable' ? 'Not translated — not available in this mode' : 'Not translated — translation failed';
      this.updateLine(u.id, { state: 'untranslated', note: why, speech: this.snap.speak ? 'skipped' : 'off', location });
      this.diag.count('translate_failed');
    }
    this.pump();
    this.flushSpeech();
  }

  /** Enqueue speech strictly in commit order; never out of order, never twice. */
  private flushSpeech(): void {
    while (this.speakCursor < this.order.length) {
      const id = this.order[this.speakCursor];
      const line = this.snap.lines.find((l) => l.id === id);
      if (!line) {
        this.speakCursor++;
        continue;
      }
      if (line.state === 'translating') return; // wait: an earlier phrase isn't ready
      this.speakCursor++;
      if (line.state !== 'translated' || !this.snap.speak || line.speech !== 'waiting') continue;
      const m = this.meta.get(id);
      if (m && this.now() - m.heardAt > this.limits.maxSpeechLagMs) {
        this.updateLine(id, { speech: 'skipped', note: 'Shown, not spoken — arrived too late' });
        this.diag.count('speech_late_skipped');
        continue;
      }
      this.enqueueSpeech(line, false);
    }
  }

  private enqueueSpeech(line: AmbientLine, replay: boolean): void {
    if (!line.translation) return;
    const m = this.meta.get(line.id);
    this.deps.playback.enqueue({
      segmentId: replay ? `${line.id}#r${++this.idSeq}` : line.id,
      text: line.translation,
      lang: this.snap.to,
      volume: this.snap.volume,
      notAfter: replay ? undefined : (m?.heardAt ?? this.now()) + this.limits.maxSpeechLagMs + 4000,
      ...(this.policy ? { ttsRoute: () => this.ttsRoute() } : {}),
    });
    if (!replay) this.updateLine(line.id, { speech: 'queued' });
  }

  private ttsRoute(): TtsRoute {
    const policy = this.policy;
    if (!policy) return 'cloud-first';
    const plan = policy.plan('tts', { deviceAvailable: Boolean(this.registry?.hasLocalVoice(this.snap.to)), cloudAvailable: true });
    this.diag.record('route', { kind: 'tts', status: plan.steps[0] ?? 'none', reason: plan.reason });
    if (!plan.steps.length || (plan.steps[0] === 'device' && plan.steps.length === 1)) return 'device-only';
    if (plan.steps[0] === 'device') return 'device-first';
    return 'cloud-first';
  }

  private onPlayback(rawId: string, status: string): void {
    if (!rawId.startsWith(ID_PREFIX)) return;
    const id = rawId.split('#')[0];
    const isReplay = rawId.includes('#');
    if (status === 'started') {
      const line = this.snap.lines.find((l) => l.id === id);
      if (line?.translation) this.spoken.push({ t: this.now(), text: line.translation });
      if (this.spoken.length > 20) this.spoken.shift();
      const m = this.meta.get(id);
      if (m && !isReplay) {
        this.diag.sample('amb_e2e_speech_ms', this.now() - m.heardAt);
        if (m.translatedAt) this.diag.sample('amb_speech_wait_ms', this.now() - m.translatedAt);
      }
      if (!isReplay) this.updateLine(id, { speech: 'playing' });
      this.commit({ playingId: id });
      // Half-duplex: no headphones → don't listen to our own speaker.
      if (!this.snap.headphones && this.asr) {
        this.stopCapture('half-duplex');
        this.commit({ heldForSpeech: true });
        this.diag.count('half_duplex_holds');
      }
      return;
    }
    if (!isReplay) {
      const map: Record<string, AmbientLine['speech']> = { ended: 'played', cancelled: 'skipped', dropped: 'skipped', failed: 'failed' };
      if (map[status]) this.updateLine(id, { speech: map[status], ...(status === 'dropped' ? { note: 'Shown, not spoken — arrived too late' } : {}) });
    }
    if (this.snap.playingId === id) this.commit({ playingId: null });
    if (!this.deps.playback.active) this.resumeCaptureIfHeld();
  }

  private resumeCaptureIfHeld(): void {
    if (!this.snap.heldForSpeech || this.disposed) return;
    if (this.resumeTimer) this.clearT(this.resumeTimer);
    this.resumeTimer = this.setT(() => {
      this.resumeTimer = null;
      if (this.deps.playback.active && !this.snap.headphones && this.snap.speak) return; // more speech queued
      this.commit({ heldForSpeech: false });
      if (this.snap.phase === 'listening' || this.snap.phase === 'starting') this.startCapture();
    }, this.limits.resumeAfterSpeechMs);
  }

  /**
   * Cancel our pending/playing speech (and, unless kept, translations in flight).
   * Capture held back for speech resumes only when explicitly asked (speech
   * turned off while listening) — never as a side effect of stop/pause/dispose.
   */
  private cancelOutput(keepTranslations = false, resumeCapture = false): void {
    if (!keepTranslations) {
      for (const c of this.inFlight.values()) c.abort();
      for (const u of this.pending) this.updateLine(u.id, { state: 'skipped', note: 'Stopped', speech: 'skipped' });
      this.pending = [];
    }
    if (this.snap.playingId || this.snap.lines.some((l) => l.speech === 'queued' || l.speech === 'playing')) this.deps.playback.cancelAll();
    // Everything not yet spoken won't be: advance the cursor past it.
    const lines = this.snap.lines.map((l) => (l.speech === 'waiting' || l.speech === 'queued' || l.speech === 'playing' ? { ...l, speech: 'skipped' as const } : l));
    this.speakCursor = this.order.length;
    this.commit({ lines, playingId: null });
    if (this.snap.heldForSpeech) {
      this.commit({ heldForSpeech: false });
      if (resumeCapture && this.snap.phase === 'listening') this.startCapture();
    }
  }

  // ===========================================================================
  // Processing policy
  // ===========================================================================

  private onPolicyChange(): void {
    const policy = this.policy;
    if (!policy || this.disposed) return;
    this.commit({ processing: { ...this.snap.processing, mode: policy.mode, network: policy.network } });
    if (policy.network === 'offline' && (this.snap.phase === 'listening' || this.snap.phase === 'starting')) {
      this.pauseFor('offline');
    } else if (policy.network !== 'offline' && this.snap.phase === 'paused' && this.snap.pauseReason === 'offline') {
      this.begin(); // the user started it; resuming after a dropped connection keeps their intent
    } else if (this.asr && this.asrGate && !policy.cloudPermitted('asr', { cloudConsent: true }).ok) {
      this.pauseFor('policy'); // cloud recognition is no longer allowed (e.g. switched to On-device)
    }
    if (!policy.cloudPermitted('translate').ok && this.snap.playingId) {
      this.deps.playback.cancelAll();
    }
    this.refreshCapabilities();
  }

  private refreshCapabilities(): void {
    if (this.disposed) return;
    const policy = this.policy;
    const reg = this.registry;
    if (!policy || !reg) {
      this.commit({ processing: { ...this.snap.processing, asr: this.deps.browser.webSpeech ? 'cloud' : 'unavailable', translate: 'cloud', speech: this.serverTts ? 'cloud' : 'device' } });
      return;
    }
    const st = (task: 'asr' | 'translate' | 'tts', deviceAvailable: boolean, cloudAvailable: boolean): CapState => {
      const p = policy.preview(task, { deviceAvailable, cloudAvailable, cloudConsent: true });
      const has = (l: string) => p.steps.includes(l as never);
      return has('device') && has('cloud') ? 'device+cloud' : has('device') ? 'device' : has('cloud') ? 'cloud' : 'unavailable';
    };
    const { from, to } = this.snap;
    this.commit({
      processing: {
        available: true,
        mode: policy.mode,
        network: policy.network,
        asr: st('asr', reg.hasLocalAsr(from), this.deps.browser.webSpeech),
        translate: st('translate', reg.canTranslate(from, to), true),
        speech: st('tts', reg.hasLocalVoice(to), this.serverTts || this.deps.browser.speechSynthesis),
      },
    });
  }

  private commit(patch: Partial<AmbientSnapshot>): void {
    if (this.disposed) return;
    this.snap = { ...this.snap, ...patch };
    for (const l of this.listeners) l(this.snap);
  }
}

function clamp01(v: number): number {
  return Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 1;
}

function defaultAudioSession(): AudioSessionLike | null {
  if (typeof navigator === 'undefined') return null;
  const s = (navigator as unknown as { audioSession?: AudioSessionLike }).audioSession;
  return s && typeof s.type === 'string' ? s : null;
}
