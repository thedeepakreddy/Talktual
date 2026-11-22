// CommunicationSession — one call, end to end, without React.
//
//   capture/ASR ──final──▶ translate (serial) ──▶ transport ──▶ peer
//   peer ──segment──▶ dedupe ─▶ (re-translate) ─▶ playback queue ─▶ speaker
//
// The session owns lifecycle (CallPhase), local pipeline activity (Activity),
// privacy gates, cancellation and cleanup. Views read `getSnapshot()` and call
// commands; they never touch adapters directly.

import { EventBus } from './eventBus';
import {
  Activity,
  CallPhase,
  EngineEvent,
  ErrorCode,
  ListeningMode,
  makeEvent,
  PeerMessage,
  PermissionState,
  SegmentRecord,
  Transport,
} from './events';
import { ACTIVITY_TRANSITIONS, PHASE_TRANSITIONS, StateMachine, TERMINAL_PHASES } from './sessionMachine';
import { newSegmentId, SegmentLedger } from './segments';
import { Diagnostics } from './diagnostics';
import { PrivacyPolicyController, disclosureFor, Disclosure } from './privacy';
import { BrowserCapabilities, degradationMessage, resolveFeatures, ResolvedFeatures } from './capabilities';
import type {
  AsrAdapter,
  AsrErrorCode,
  AsrFactory,
  CapturePort,
  ConnectFn,
  PlaybackPort,
  RecognizedSegment,
  ServerCapabilities,
  StorageLike,
  Translator,
} from './ports';
import type { CallHandle, TransportStatus } from '../lib/webrtc';
import { VoiceMirrorController, type VoiceTapPort } from '../voicemirror/controller.ts';
import {
  CONVERSATION_CAP,
  DEFAULT_CONVERSATION_CONFIG,
  LatencyMonitor,
  ListenerPolicy,
  OutboundQueue,
  Segmenter,
  type CommitReason,
  type ConversationConfig,
  type Unit,
} from '../conversation';
import type { PlaybackItem, TranslationRequest, TranslationResult, TtsRoute } from './ports';
import { anySignal, PolicyBlockedError, ProcessingPolicy, PROCESSING_MODES, type InferenceTask, type NetworkState, type ProcessingMode, type RouteReason } from '../privacy/policy';
import { browserNetworkSource, NetworkMonitor, type NetworkSource } from '../privacy/network';
import { RuntimeRegistry, type RegistryLimits } from '../local/registry';
import { PhrasebookRuntime } from '../local/phrasebook';
import type { AsrRuntime, RuntimeInfo, TranslateRuntime, VoiceRuntime } from '../local/types';
import {
  ACI_CAP,
  IntelligenceController,
  type AciLimits,
  type AciPrefs,
  type ContextSummary,
  type OutboundDecision,
  type Suggestion,
} from '../intelligence';

export const PARTIAL_SEND_INTERVAL_MS = 180;
/** Phase 6 wire capability: the peer honours `priv: 'device'` on segments/partials. */
export const PRIV_CAP = 'priv1';
const perfNow = () => globalThis.performance?.now?.() ?? Date.now();
export const PARTIAL_TRANSLATE_DEBOUNCE_MS = 400;
export const NOTICE_MS = 5000;
export const MAX_TRANSCRIPT = 500;

export interface SessionConfig {
  roomCode: string;
  /** BCP-47 language the local user speaks (and hears). */
  lang: string;
  userName: string;
}

export interface SessionDeps {
  connect: ConnectFn;
  signalingUrl: string;
  createAsr: AsrFactory;
  translate: Translator;
  /** Live-caption translation of the peer's interim text (abortable). */
  translateCaption: (text: string, from: string, to: string, signal: AbortSignal) => Promise<string>;
  playback: PlaybackPort;
  capture: CapturePort;
  getServerCapabilities: () => Promise<ServerCapabilities>;
  browser: BrowserCapabilities;
  storage: StorageLike | null;
  /** Called synchronously inside the talk gesture (mobile audio unlock). */
  onUserGesture?: () => void;
  /** VoiceMirror (Phase 3): present only when the feature flag is on. */
  voiceTap?: VoiceTapPort;
  /**
   * Phase 4 conversation layer (turn/pause detection, self-repair, provisional
   * output, interruption/overlap handling, backpressure). Off when absent —
   * the engine then behaves exactly as in Phase 3. Production wiring turns it
   * on unless VITE_CONVERSATION_LAYER=0.
   */
  conversation?: boolean | Partial<ConversationConfig>;
  /**
   * Phase 5 Autonomous Communication Intelligence (session-scoped context,
   * context-aware translation, notes, signals, suggestions). Off when absent —
   * exact Phase 4 behaviour. Production turns it on unless
   * VITE_COMMUNICATION_INTELLIGENCE=0.
   */
  intelligence?: boolean | { prefs?: Partial<AciPrefs>; limits?: Partial<AciLimits> };
  /**
   * Phase 6 processing modes (cloud / hybrid / on-device only), central policy,
   * local runtimes and offline handling. Off when absent — exact Phase 5
   * behaviour. Production turns it on unless VITE_PROCESSING_MODES=0.
   */
  processing?: boolean | ProcessingOptions;
  /** Phase 6: drops client-side caches (translation cache) — called on session end and mode change. */
  clearCaches?: () => void;
  now?: () => number;
  setTimeout?: typeof setTimeout;
  clearTimeout?: typeof clearTimeout;
}

export interface ProcessingOptions {
  /** Initial mode (otherwise the user's saved choice, otherwise 'cloud'). */
  mode?: ProcessingMode;
  /** Extra on-device runtimes (the bundled phrasebook is always registered). */
  translators?: TranslateRuntime[];
  voices?: VoiceRuntime[];
  asr?: AsrRuntime[];
  network?: NetworkSource;
  registryLimits?: RegistryLimits;
}

export const PROCESSING_STORAGE_KEY = 'talktual.processing.v1';

export type CapabilityState = 'device' | 'cloud' | 'device+cloud' | 'unavailable';
export interface CapabilityRow {
  task: 'speech-input' | 'translation' | 'speech-output' | 'captions' | 'typed-input' | 'assistance';
  state: CapabilityState;
  /** Short, non-sensitive explanation. */
  detail: string;
}

export interface Notice {
  code: ErrorCode | 'info' | 'untranslated-out' | 'untranslated-in' | 'peer-away' | 'degraded' | 'slow' | 'local-only' | 'offline';
  message: string;
  sticky: boolean;
}

export interface SessionSnapshot {
  phase: CallPhase;
  phaseReason: string | null;
  activity: Activity;
  transport: Transport;
  lang: string;
  peer: { name: string | null; lang: string | null };
  mode: ListeningMode;
  consent: { route: string | null; granted: boolean; prompt: boolean; disclosure: Disclosure | null };
  permission: PermissionState;
  /** True whenever audio may be leaving the device for recognition. */
  capturing: boolean;
  holding: boolean;
  hearingVoice: boolean;
  playing: boolean;
  translating: number;
  partial: { local: string; peer: string };
  segments: SegmentRecord[];
  notice: Notice | null;
  features: ResolvedFeatures;
  lastLatencyMs: number | null;
  connectedAt: number | null;
  reconnectAttempt: number;
  /** VoiceMirror (Phase 3). */
  voice: {
    available: boolean;
    /** Disclosure for sharing voice characteristics is open. */
    prompt: boolean;
    sharing: boolean;
    descriptorSent: boolean;
    peerActive: boolean;
  };
  /** Phase 4 conversation layer. */
  conversation: {
    enabled: boolean;
    /** The peer understands provisional/interrupt/floor messages. */
    peerCapable: boolean;
    /** Translation is slow: provisional output off, phrases merged. */
    degraded: boolean;
    /** The local unit currently held as a pause candidate (shown, not yet sent as final). */
    held: { id: string; text: string; provisional: boolean } | null;
    /** Incoming segments waiting because the local user is talking. */
    overlapHeld: number;
    /** The peer currently holds the floor. */
    peerTalking: boolean;
    /** Phase 4 drafts (provisional translations) are sent; user-controllable (disclosed). */
    drafts: boolean;
  };
  /** Phase 6 processing modes. */
  processing: {
    available: boolean;
    mode: ProcessingMode;
    network: NetworkState;
    capabilities: CapabilityRow[];
    /** Functions unavailable right now (disclosed). */
    degraded: boolean;
    /** Discovered on-device runtimes (for the privacy sheet). */
    runtimes: RuntimeInfo[];
    /** The other person is in on-device mode. */
    peerPrivate: boolean;
  };
  /** Phase 5 communication intelligence. */
  intelligence: {
    available: boolean;
    prefs: AciPrefs | null;
    suggestions: Suggestion[];
    /** Inspectable, correctable context summary (no raw turns). */
    context: ContextSummary | null;
    /** The peer's app explains its context-assisted translations to us. */
    peerCapable: boolean;
  };
}

export class CommunicationSession {
  readonly events: EventBus;
  private readonly diag: Diagnostics;
  private readonly ledger = new SegmentLedger(MAX_TRANSCRIPT);
  private readonly privacy: PrivacyPolicyController;
  private readonly phase: StateMachine<CallPhase>;
  private readonly activity: StateMachine<Activity>;
  private readonly now: () => number;
  private readonly setT: typeof setTimeout;
  private readonly clearT: typeof clearTimeout;

  private snap: SessionSnapshot;
  private subscribers = new Set<() => void>();

  private call: CallHandle | null = null;
  private asr: AsrAdapter | null = null;
  private server: ServerCapabilities | null = null;
  private started = false;
  private disposed = false;
  private consentDeclined = false;
  private micEnabled = true;
  private pausedForPlayback = false;

  private sendChain: Promise<void> = Promise.resolve();
  /** Aborts in-flight translations on leave/teardown. */
  private work = new AbortController();
  private lastPartialSentAt = 0;
  private partialTimer: ReturnType<typeof setTimeout> | null = null;
  private partialAbort: AbortController | null = null;
  private noticeTimer: ReturnType<typeof setTimeout> | null = null;
  private unsubs: Array<() => void> = [];
  private voice: VoiceMirrorController | null = null;

  // --- Phase 4 conversation layer (null when disabled) ---
  private readonly conv: ConversationConfig | null;
  private readonly seg: Segmenter | null = null;
  private readonly listen: ListenerPolicy | null = null;
  private readonly outq: OutboundQueue | null = null;
  private readonly lat: LatencyMonitor | null = null;
  private turnId: string | null = null;
  private floorOpen = false;
  private floorTimer: ReturnType<typeof setTimeout> | null = null;
  private prov: { id: string; controller: AbortController } | null = null;
  private lastProvAt = -Infinity;
  private provSent = new Set<string>();
  private peerCaps = false;
  private wasTalking = false;
  private drafts = true;

  // --- Phase 6 processing policy (null when disabled) ---
  private policy: ProcessingPolicy | null = null;
  private registry: RuntimeRegistry | null = null;
  private netSource: NetworkSource | null = null;
  private netmon: NetworkMonitor | null = null;
  private peerPrivCap = false;
  private asrGate: { signal: AbortSignal; end: () => void } | null = null;

  // --- Phase 5 communication intelligence (null when disabled) ---
  private aci: IntelligenceController | null = null;
  private peerAci = false;
  private aciTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    private config: SessionConfig,
    private readonly deps: SessionDeps
  ) {
    this.now = deps.now ?? (() => Date.now());
    this.setT = deps.setTimeout ?? setTimeout.bind(globalThis);
    this.clearT = deps.clearTimeout ?? clearTimeout.bind(globalThis);
    this.diag = new Diagnostics(300, 200, this.now);
    this.events = new EventBus((err) => {
      this.diag.count('listener_errors');
      console.error('engine listener failed:', err);
    });
    this.privacy = new PrivacyPolicyController(deps.storage);

    this.phase = new StateMachine<CallPhase>(
      'idle',
      PHASE_TRANSITIONS,
      (from, to, reason) => {
        this.diag.record('phase', { from, to, reason });
        this.emit({ type: 'session.phase', from, to, reason });
      },
      (from, to) => {
        this.diag.record('illegal_phase', { from, to });
        this.diag.count('illegal_transitions');
      }
    );
    this.activity = new StateMachine<Activity>(
      'idle',
      ACTIVITY_TRANSITIONS,
      (from, to, reason) => {
        this.diag.record('activity', { from, to, reason });
        this.emit({ type: 'session.activity', from, to, reason });
      },
      (from, to) => {
        this.diag.record('illegal_activity', { from, to });
        this.diag.count('illegal_transitions');
      }
    );

    const features = resolveFeatures(deps.browser, null);
    this.snap = {
      phase: 'idle',
      phaseReason: null,
      activity: 'idle',
      transport: 'none',
      lang: config.lang,
      peer: { name: null, lang: null },
      mode: 'push-to-talk',
      consent: { route: features.route, granted: this.privacy.hasConsent(features.route), prompt: false, disclosure: null },
      permission: 'unknown',
      capturing: false,
      holding: false,
      hearingVoice: false,
      playing: false,
      translating: 0,
      partial: { local: '', peer: '' },
      segments: [],
      notice: null,
      features,
      lastLatencyMs: null,
      connectedAt: null,
      reconnectAttempt: 0,
      voice: { available: Boolean(deps.voiceTap), prompt: false, sharing: false, descriptorSent: false, peerActive: false },
      conversation: { enabled: Boolean(deps.conversation), peerCapable: false, degraded: false, held: null, overlapHeld: 0, peerTalking: false, drafts: true },
      intelligence: { available: Boolean(deps.intelligence), prefs: null, suggestions: [], context: null, peerCapable: false },
      processing: { available: false, mode: 'cloud', network: 'online', capabilities: [], degraded: false, runtimes: [], peerPrivate: false },
    };
    if (deps.processing) {
      const opts: ProcessingOptions = typeof deps.processing === 'object' ? deps.processing : {};
      let saved: string | null = null;
      try {
        saved = deps.storage?.getItem(PROCESSING_STORAGE_KEY) ?? null;
      } catch {
        saved = null;
      }
      const mode = opts.mode ?? (PROCESSING_MODES.includes(saved as ProcessingMode) ? (saved as ProcessingMode) : 'cloud');
      this.policy = new ProcessingPolicy(mode, this.now);
      this.registry = new RuntimeRegistry(
        { now: this.now, setTimeout: (fn, ms) => this.setT(fn, ms), clearTimeout: (t) => this.clearT(t as ReturnType<typeof setTimeout>) },
        opts.registryLimits
      );
      this.registry.addTranslator(new PhrasebookRuntime(this.now));
      for (const t of opts.translators ?? []) this.registry.addTranslator(t);
      for (const v of opts.voices ?? []) this.registry.addVoice(v);
      for (const a of opts.asr ?? []) this.registry.addAsr(a);
      this.netSource = opts.network ?? browserNetworkSource(typeof window !== 'undefined' ? window : undefined);
      this.snap = { ...this.snap, processing: { ...this.snap.processing, available: true, mode } };
    }
    if (deps.intelligence) {
      const opts = typeof deps.intelligence === 'object' ? deps.intelligence : {};
      this.aci = new IntelligenceController(this.now, opts);
      this.aci.setLangs(config.lang, null);
      this.snap = { ...this.snap, intelligence: { ...this.snap.intelligence, prefs: this.aci.getPrefs(), context: this.aci.summary() } };
    }
    this.conv = deps.conversation ? { ...DEFAULT_CONVERSATION_CONFIG, ...(typeof deps.conversation === 'object' ? deps.conversation : {}) } : null;
    if (this.conv) {
      const cfg = this.conv;
      this.listen = new ListenerPolicy(cfg.maxInterruptedTurns);
      this.outq = new OutboundQueue(cfg.maxUnitChars);
      this.lat = new LatencyMonitor(cfg.degradedLatencyMs);
      this.seg = new Segmenter(
        config.lang,
        cfg,
        {
          onHold: (u) => this.onUnitHeld(u, 'held'),
          onRevise: (u) => this.onUnitHeld(u, 'revised'),
          onCommit: (u, reason) => this.onUnitCommitted(u, reason),
          onDiscard: (u) => this.onUnitDiscarded(u),
          onDrop: (kind) => this.diag.count(kind === 'filler' ? 'fillers_dropped' : 'noise_dropped'),
          onIllegal: (from, to) => {
            this.diag.record('illegal_unit', { from, to });
            this.diag.count('illegal_transitions');
          },
        },
        { now: this.now, setTimeout: (fn, ms) => this.setT(fn, ms), clearTimeout: (t) => this.clearT(t as ReturnType<typeof setTimeout>) },
        newSegmentId
      );
    }
    if (deps.voiceTap) {
      this.voice = new VoiceMirrorController({
        tap: deps.voiceTap,
        send: (m) => this.call?.send(m),
        // Enum status only: no voice-derived numbers ever reach diagnostics.
        diag: (status) => this.diag.record('voicemirror', { status }),
        onChange: () => {
          if (this.voice) this.commit({ voice: { ...this.snap.voice, ...this.voice.state } });
        },
      });
    }
  }

  // ==========================================================================
  // VoiceMirror (Phase 3) — opt-in per call, never persisted
  // ==========================================================================

  /** Opens the VoiceMirror disclosure (or turns it off if it is on). */
  toggleVoiceMirror(): void {
    if (!this.voice || this.isTerminal) return;
    if (this.voice.state.sharing) {
      this.voice.optOut();
      return;
    }
    this.commit({ voice: { ...this.snap.voice, prompt: true } });
  }

  acceptVoiceMirror(): void {
    if (!this.voice || this.isTerminal) return;
    this.commit({ voice: { ...this.snap.voice, prompt: false } });
    this.voice.optIn();
    // Already talking (hands-free): start analysing now.
    if (this.asr?.active) this.voice.onCaptureStart();
  }

  declineVoiceMirror(): void {
    if (!this.voice) return;
    this.commit({ voice: { ...this.snap.voice, prompt: false } });
  }

  /** Revocation: stops analysis, deletes local data, tells the peer to delete theirs. */
  stopVoiceMirror(): void {
    this.voice?.optOut();
  }

  /** For tests/diagnostics: is any voice-derived data held in memory? */
  holdsVoiceData(): boolean {
    return this.voice?.holdsVoiceData() ?? false;
  }

  // ==========================================================================
  // Public surface
  // ==========================================================================

  getSnapshot = (): SessionSnapshot => this.snap;

  subscribe = (fn: () => void): (() => void) => {
    this.subscribers.add(fn);
    return () => this.subscribers.delete(fn);
  };

  getDiagnostics() {
    return this.diag.snapshot();
  }

  get isTerminal(): boolean {
    return TERMINAL_PHASES.has(this.phase.state);
  }

  /** Connects the call. Idempotent. */
  start(): void {
    if (this.started || this.disposed) return;
    this.started = true;
    this.toPhase('connecting');

    this.unsubs.push(this.deps.playback.onStatus((id, status, engine) => this.onPlaybackStatus(id, status, engine)));
    this.unsubs.push(this.deps.capture.onDeviceLost(() => this.onAsrError('device-lost', 'Microphone disconnected.', true)));

    this.deps
      .getServerCapabilities()
      .then((caps) => {
        if (this.disposed) return;
        this.server = caps;
        this.refreshFeatures();
      })
      .catch(() => {
        /* getServerCapabilities already degrades to defaults */
      });

    this.deps
      .capture.permission()
      .then((state) => {
        if (!this.disposed) this.setPermission(state);
      })
      .catch(() => {});

    if (this.policy && this.registry && this.netSource) {
      const policy = this.policy;
      this.netmon = new NetworkMonitor(this.netSource, this.now, (s) => policy.setNetwork(s));
      this.unsubs.push(policy.onChange(() => this.onPolicyChange()));
      policy.setNetwork(this.netmon.state);
      this.registry
        .discover()
        .then(() => this.refreshProcessing())
        .catch(() => this.refreshProcessing());
      this.refreshProcessing();
    }

    this.call = this.deps.connect(this.config.roomCode, this.deps.signalingUrl, {
      onMessage: (msg) => this.onPeerMessage(msg),
      onStatus: (s) => this.onTransportStatus(s as TransportStatus),
      onTransport: (t) => {
        if (this.isTerminal) return;
        this.commit({ transport: t });
        this.emit({ type: 'transport.changed', transport: t });
        this.diag.record('transport', { transport: t });
      },
      onReconnectAttempt: (attempt) => {
        this.commit({ reconnectAttempt: attempt });
        this.emit({ type: 'recovery', kind: 'reconnecting', attempt });
        this.diag.record('reconnect_attempt', { attempt });
      },
    });
    this.sendHello();
  }

  /** User accepted the processing disclosure. */
  grantConsent(): void {
    const route = this.snap.features.route;
    if (!route || this.isTerminal) return;
    this.privacy.grant(route);
    this.consentDeclined = false;
    this.diag.record('consent', { route, status: 'granted' });
    this.commit({ consent: { ...this.snap.consent, route, granted: true, prompt: false } });
    this.emitPrivacy();
    if (this.activity.state === 'idle') this.activity.to('ready', 'consent');
    this.commitActivity();
  }

  /** User dismissed the disclosure. Listening to the peer continues. */
  declineConsent(): void {
    this.consentDeclined = true;
    // Declining processing also withdraws any conversation context gathered so far.
    if (this.aci) {
      this.aci.reset();
      this.refreshAci();
    }
    this.diag.record('consent', { route: this.snap.features.route ?? 'none', status: 'declined' });
    this.commit({ consent: { ...this.snap.consent, prompt: false } });
  }

  /** Opens the disclosure again (e.g. from a settings/info control). */
  showDisclosure(): void {
    this.promptConsent();
  }

  pressTalk(): void {
    if (!this.canAcceptTalk() || this.privacy.mode !== 'push-to-talk' || this.snap.holding) return;
    if (!this.privacy.mayCapture(this.snap.features.route)) {
      this.promptConsent();
      return;
    }
    // Phase 6: in on-device/offline mode, speech input may be unavailable → fail closed, offer typing.
    if (this.policy && !this.asrPlan(false).steps.length) {
      this.setNotice({
        code: this.policy.mode === 'local-only' ? 'local-only' : 'offline',
        message:
          this.policy.mode === 'local-only'
            ? 'Speech recognition isn’t available on this device in On-device mode — type your message instead.'
            : 'You’re offline — speech recognition needs a connection. Type your message instead.',
        sticky: false,
      });
      return;
    }
    this.deps.onUserGesture?.();
    // Phase 4: taking the floor while the peer is mid-turn (or being played)
    // interrupts that turn — its later segments are kept but not spoken.
    let interrupted: string[] = [];
    if (this.listen) {
      interrupted = this.listen.bargeIn(this.now(), this.deps.playback.active);
      if (this.peerCaps) for (const turn of interrupted) this.call?.send({ t: 'interrupt', turn });
      if (interrupted.length) {
        this.diag.count('turns_interrupted', interrupted.length);
        this.markTurns('peer', interrupted);
      }
    }
    if (this.deps.playback.active) this.interrupt('barge-in', interrupted.length);
    else if (interrupted.length) this.emit({ type: 'interruption', kind: 'barge-in', cancelledSegments: 0, interruptedTurns: interrupted.length });
    this.commit({ holding: true, partial: { ...this.snap.partial, local: '' } });
    if (this.seg) {
      this.closeFloor();
      this.openFloor();
    }
    this.syncTalking();
    this.startAsr('press');
  }

  releaseTalk(): void {
    if (!this.snap.holding) return;
    this.commit({ holding: false, hearingVoice: false });
    // End-of-turn cue: whatever is held commits once the recognizer flushes.
    this.seg?.release();
    this.closeFloor();
    this.syncTalking();
    if (this.privacy.mode === 'push-to-talk' && this.asr?.active) {
      this.activity.to('processing', 'release');
      this.commitActivity();
      this.asr.stop();
    } else if (this.activity.state === 'listening') {
      this.activity.to('ready', 'release');
      this.commitActivity();
    }
    this.call?.send({ t: 'partial', text: '', lang: this.config.lang });
  }

  setListeningMode(mode: ListeningMode): void {
    if (this.isTerminal || mode === this.privacy.mode) return;
    if (mode === 'continuous') {
      if (!this.snap.features.continuous || !this.micEnabled) return;
      if (!this.privacy.mayCapture(this.snap.features.route)) {
        this.promptConsent();
        return;
      }
    }
    this.stopAsr('abort', 'mode-change');
    this.privacy.setMode(mode);
    this.diag.record('mode', { mode });
    this.commit({ mode, holding: false });
    if (mode === 'continuous' && this.micEnabled && this.phase.state === 'connected') this.startAsr('continuous');
    this.emitPrivacy();
  }

  /** Mic off = stop all capture now; mic on = back to ready (or listening in continuous). */
  setMicEnabled(on: boolean): void {
    if (this.isTerminal || on === this.micEnabled) return;
    this.micEnabled = on;
    if (!on) {
      this.commit({ holding: false });
      this.stopAsr('abort', 'user-stop');
      this.deps.capture.release();
      this.activity.to('stopped', 'user-stop');
      this.emit({ type: 'interruption', kind: 'stop', cancelledSegments: 0 });
    } else {
      // Retry after a denial/device loss: re-check rather than trust the old state.
      if (this.snap.permission === 'denied') this.setPermission('unknown');
      if (this.snap.notice && ['permission-denied', 'device-lost', 'capture-unavailable'].includes(this.snap.notice.code)) {
        this.clearNotice();
      }
      this.deps.capture
        .permission()
        .then((state) => {
          if (!this.isTerminal && state !== 'unknown') this.setPermission(state);
        })
        .catch(() => {});
      this.activity.to(this.privacy.hasConsent(this.snap.features.route) ? 'ready' : 'idle', 'user-start');
      if (this.privacy.mode === 'continuous' && this.phase.state === 'connected') this.startAsr('continuous');
    }
    this.commitActivity();
    this.emitPrivacy();
  }

  setLanguage(lang: string): void {
    if (this.isTerminal || lang === this.config.lang) return;
    const wasContinuous = this.privacy.mode === 'continuous' && this.asr?.active;
    this.stopAsr('abort', 'language-change');
    this.config = { ...this.config, lang };
    this.seg?.setLang(lang);
    this.aci?.setLangs(lang, this.snap.peer.lang);
    this.commit({ lang, holding: false });
    this.sendHello();
    if (wasContinuous) this.startAsr('continuous');
  }

  /** Intentional hang-up. */
  leave(): void {
    if (this.isTerminal) return;
    this.toPhase('ended', 'local-leave');
  }

  /** Releases everything. Safe to call more than once and after leave(). */
  dispose(): void {
    if (this.disposed) return;
    if (!this.isTerminal) this.toPhase('ended', 'disposed');
    this.teardown();
    this.disposed = true;
    this.subscribers.clear();
    this.events.clear();
  }

  // ==========================================================================
  // Phase handling
  // ==========================================================================

  private toPhase(to: CallPhase, reason?: string): void {
    if (!this.phase.to(to, reason)) return;
    const patch: Partial<SessionSnapshot> = { phase: to, phaseReason: reason ?? null };
    if (to === 'connected' && this.snap.connectedAt === null) patch.connectedAt = this.now();
    if (to === 'connected') patch.reconnectAttempt = 0;
    this.commit(patch);
    if (TERMINAL_PHASES.has(to)) this.teardown();
  }

  private onTransportStatus(status: TransportStatus): void {
    if (this.isTerminal) return;
    switch (status) {
      case 'waiting-for-peer':
        this.toPhase('waiting-for-peer');
        break;
      case 'connected': {
        const wasReconnecting = this.phase.state === 'reconnecting';
        this.toPhase('connected');
        // Floor messages are not resent after an outage; don't trust a stale "open".
        this.listen?.resetFloor();
        if (wasReconnecting) {
          this.emit({ type: 'recovery', kind: 'reconnected' });
          this.diag.record('reconnected', {});
        }
        if (this.snap.notice?.code === 'peer-away' || this.snap.notice?.code === 'connection-lost') this.clearNotice();
        this.sendHello();
        this.voice?.onConnected();
        this.refreshFeatures();
        if (this.privacy.mode === 'continuous' && this.micEnabled && !this.asr?.active) this.startAsr('continuous');
        break;
      }
      case 'peer-away':
        this.toPhase('waiting-for-peer', 'peer-away');
        this.emit({ type: 'recovery', kind: 'peer-away' });
        this.setNotice({ code: 'peer-away', message: `${this.snap.peer.name ?? 'They'} lost connection — waiting for them to come back…`, sticky: true });
        this.pauseCaptureWhileAlone();
        break;
      case 'peer-back':
        this.emit({ type: 'recovery', kind: 'peer-back' });
        break;
      case 'reconnecting':
        this.toPhase('reconnecting');
        this.setNotice({ code: 'connection-lost', message: 'Connection lost — reconnecting…', sticky: true });
        this.pauseCaptureWhileAlone();
        break;
      case 'peer-left':
        this.toPhase('ended', 'peer-left');
        break;
      case 'expired':
        this.toPhase('ended', 'expired');
        break;
      case 'session-full':
        this.fail('session-full', 'That channel is already full.');
        break;
      case 'invalid-session':
        this.fail('session-invalid', 'That channel code isn’t valid.');
        break;
      case 'resume-failed':
        this.emit({ type: 'recovery', kind: 'gave-up' });
        this.fail('resume-failed', 'The call ended while you were offline.');
        break;
      case 'disconnected':
        this.emit({ type: 'recovery', kind: 'gave-up' });
        this.fail('connection-lost', 'Disconnected. Leave and start a new channel to continue.');
        break;
      case 'error':
        this.fail('server-error', 'Couldn’t reach the server.');
        break;
    }
  }

  private fail(code: ErrorCode, message: string): void {
    this.setNotice({ code, message, sticky: true });
    this.emit({ type: 'error', code, fatal: true, message });
    this.toPhase('failed', code);
  }

  /** While nobody can hear us, don't keep listening. */
  private pauseCaptureWhileAlone(): void {
    if (this.asr?.active) this.stopAsr('abort', 'peer-unavailable');
    if (this.snap.holding) this.commit({ holding: false });
  }

  private teardown(): void {
    if (this.partialTimer) this.clearT(this.partialTimer);
    if (this.noticeTimer) this.clearT(this.noticeTimer);
    this.noticeTimer = null;
    this.partialAbort?.abort();
    this.work.abort();
    const cancelled = this.deps.playback.cancelAll();
    this.stopAsr('abort', 'teardown', true);
    // Phase 4: conversation context is session-scoped — drop all of it.
    this.prov?.controller.abort();
    this.prov = null;
    this.seg?.dispose();
    this.listen?.clear();
    this.outq?.clear();
    this.lat?.reset();
    this.provSent.clear();
    // Phase 5: session context is ephemeral — gone at session end.
    this.aci?.reset();
    // Phase 6: local runtimes unloaded, caches and the routing audit dropped.
    if (this.policy) {
      this.registry?.dispose();
      this.netmon?.dispose();
      this.netmon = null;
      this.policy.clearAudit();
      this.deps.clearCaches?.();
    }
    if (this.aciTimer) this.clearT(this.aciTimer);
    this.aciTimer = null;
    if (this.floorTimer) this.clearT(this.floorTimer);
    this.floorTimer = null;
    this.floorOpen = false;
    this.turnId = null;
    this.voice?.dispose();
    this.deps.capture.release();
    this.unsubs.forEach((u) => u());
    this.unsubs = [];
    if (this.call) {
      const c = this.call;
      this.call = null;
      c.close();
    }
    this.emit({ type: 'interruption', kind: 'leave', cancelledSegments: cancelled });
    this.commit({
      holding: false,
      hearingVoice: false,
      playing: false,
      capturing: false,
      translating: 0,
      partial: { local: '', peer: '' },
      conversation: { ...this.snap.conversation, held: null, overlapHeld: 0, peerTalking: false },
      ...(this.aci ? { intelligence: { ...this.snap.intelligence, suggestions: [], context: this.aci.summary() } } : {}),
    });
    this.emitPrivacy();
  }

  // ==========================================================================
  // Capture / ASR
  // ==========================================================================

  private canAcceptTalk(): boolean {
    return (
      !this.disposed &&
      this.phase.state === 'connected' &&
      this.micEnabled &&
      this.snap.features.canTalk &&
      this.snap.permission !== 'denied'
    );
  }

  private startAsr(reason: 'press' | 'continuous'): void {
    if (!this.privacy.mayCapture(this.snap.features.route) || !this.snap.features.stt) return;
    if (this.pausedForPlayback && reason === 'continuous') return;
    let asrOpts: { processLocally?: boolean } | undefined;
    if (this.policy && !this.asr) {
      const plan = this.asrPlan(true);
      if (!plan.steps.length) return;
      if (plan.steps[0] === 'device') {
        asrOpts = { processLocally: true };
        this.policy.noteDevice('asr', plan.reason);
      } else {
        try {
          const gate = this.policy.beginRemote('asr', { cloudConsent: this.privacy.hasConsent(this.snap.features.route) });
          this.asrGate = gate;
          gate.signal.addEventListener('abort', () => {
            if (this.asrGate === gate) this.stopAsr('abort', 'policy');
          });
        } catch (e) {
          if (e instanceof PolicyBlockedError) return;
          throw e;
        }
      }
      this.diag.record('route', { kind: 'asr', status: plan.steps[0], reason: plan.reason });
    }
    if (!this.asr) {
      // Callbacks from a recognizer that is no longer current (aborted by
      // stop/leave/mode change) are ignored: nothing captured after a stop
      // can be sent, even if an adapter misbehaves.
      let created: AsrAdapter | null = null;
      const current = () => created !== null && this.asr === created;
      created = this.deps.createAsr(this.config.lang, this.privacy.mode, {
        onFinal: (seg) => {
          if (current()) this.onLocalFinal(seg);
          else this.diag.count('stale_asr_results_dropped');
        },
        onPartial: (text) => {
          if (current()) this.onLocalPartial(text);
        },
        onSpeechActivity: (active) => {
          if (current()) this.commit({ hearingVoice: active && (this.snap.holding || this.privacy.mode === 'continuous') });
        },
        onError: (code, message, fatal) => {
          if (current()) this.onAsrError(code, message, fatal);
        },
        onEnd: () => {
          if (current()) this.onAsrEnd();
        },
      }, asrOpts);
      this.asr = created;
    }
    if (this.activity.state === 'idle' || this.activity.state === 'stopped') this.activity.to('ready', 'start');
    if (this.snap.permission !== 'granted') this.activity.to('permission-pending', reason);
    this.commitActivity();
    const asr = this.asr;
    asr
      .start()
      .then(() => {
        if (this.asr !== asr || !asr.active) return;
        if (this.snap.permission !== 'granted') this.setPermission('granted');
        this.activity.to('listening', reason);
        this.commitActivity();
        this.emitPrivacy();
        // VoiceMirror analyses only while capture is active (same gate as ASR).
        this.voice?.onCaptureStart();
      })
      .catch(() => {
        /* adapter reports failures through onError */
      });
    this.emitPrivacy();
  }

  /** `abort` discards; `stop` flushes the last phrase first. */
  private stopAsr(how: 'stop' | 'abort', reason: string, disposing = false): void {
    const asr = this.asr;
    this.asrGate?.end();
    this.asrGate = null;
    if (!asr) return;
    this.voice?.onCaptureStop();
    if (how === 'abort' || disposing) {
      this.asr = null;
      asr.abort();
      asr.dispose();
      // Anything held but not yet final was never meant to be sent.
      this.seg?.cancel();
      this.closeFloor();
    } else {
      asr.stop();
    }
    this.diag.record('asr_stop', { kind: how, reason });
    if (!disposing && (this.activity.state === 'listening' || this.activity.state === 'processing' || this.activity.state === 'permission-pending')) {
      this.activity.to('ready', reason);
    }
    this.commit({ partial: { ...this.snap.partial, local: '' }, hearingVoice: false });
    this.commitActivity();
    this.emitPrivacy();
  }

  private onAsrEnd(): void {
    if (this.isTerminal) return;
    this.voice?.onCaptureStop();
    this.seg?.endOfInput();
    if (this.privacy.mode === 'continuous') this.closeFloor();
    if (this.activity.state === 'processing') this.activity.to('ready', 'flushed');
    this.commitActivity();
    this.emitPrivacy();
  }

  private onAsrError(code: AsrErrorCode, message: string, fatal: boolean): void {
    if (this.isTerminal) return;
    this.diag.record('asr_error', { code, fatal });
    if (code === 'permission-denied' || code === 'device-lost' || code === 'capture-unavailable') {
      this.stopAsr('abort', code);
      if (code === 'permission-denied') this.setPermission('denied');
      this.activity.to('stopped', code);
      this.commit({ holding: false });
      this.commitActivity();
      const errCode: ErrorCode = code;
      const friendly =
        code === 'permission-denied'
          ? 'Microphone access is blocked. Allow it in your browser’s site settings, then try again. You can still hear the other person.'
          : code === 'device-lost'
            ? 'Your microphone was disconnected. Reconnect it and press to talk again.'
            : 'No microphone is available. You can still hear the other person.';
      this.setNotice({ code: errCode, message: friendly, sticky: true });
      this.emit({ type: 'error', code: errCode, fatal: true, message: friendly });
      // The mic is now off. Turning it back on (setMicEnabled(true)) is the
      // retry path: it re-checks the permission and returns to ready.
      this.micEnabled = false;
      this.emitPrivacy();
      return;
    }
    const msg = code === 'no-result' ? 'Nothing was transcribed — try holding a moment longer.' : message;
    this.setNotice({ code: 'stt-failed', message: msg, sticky: false });
    this.emit({ type: 'error', code: 'stt-failed', fatal: false, message: msg });
    if (fatal) {
      this.stopAsr('abort', 'stt-fatal');
    }
  }

  // ==========================================================================
  // Outgoing pipeline
  // ==========================================================================

  private onLocalPartial(text: string): void {
    if (this.isTerminal) return;
    if (!(this.snap.holding || this.privacy.mode === 'continuous' || this.activity.state === 'processing')) return;
    this.commit({ partial: { ...this.snap.partial, local: text } });
    this.emit({ type: 'transcript.partial', speaker: 'local', text, lang: this.config.lang });
    if (this.seg && text) {
      this.touchContinuousFloor();
      this.seg.partial(text);
    }
    const now = this.now();
    if (!text || now - this.lastPartialSentAt < PARTIAL_SEND_INTERVAL_MS) return;
    this.lastPartialSentAt = now;
    this.call?.send({ t: 'partial', text, lang: this.config.lang, ...(this.peerCaps && this.turnId ? { turn: this.turnId } : {}), ...(this.privFlag() ? { priv: 'device' as const } : {}) });
  }

  private onLocalFinal(seg: RecognizedSegment): void {
    if (this.isTerminal || !seg.text.trim()) return;
    if (seg.promoted) this.diag.count('asr_interim_promoted');
    if (this.seg) {
      this.touchContinuousFloor();
      this.commit({ partial: { ...this.snap.partial, local: '' } });
      this.seg.final(seg.text, seg.confidence);
      return;
    }
    const id = newSegmentId();
    const seq = this.ledger.nextSeq();
    const sourceLang = this.config.lang;
    const targetLang = this.snap.peer.lang ?? sourceLang;
    const record: SegmentRecord = {
      id,
      seq,
      speaker: 'local',
      speakerName: this.config.userName,
      original: seg.text.trim(),
      translated: '',
      sourceLang,
      targetLang,
      translation: 'pending',
      createdAt: this.now(),
    };
    this.addSegment(record);
    this.diag.record('seg_final', { segmentId: id, engine: seg.engine, ms: seg.endedAt - seg.startedAt });
    this.commit({ translating: this.snap.translating + 1, partial: { ...this.snap.partial, local: '' } });

    const finalAt = this.now();
    const signal = this.work.signal;
    // Serial: translations finish in the order the words were spoken.
    this.sendChain = this.sendChain
      .then(async () => {
        if (signal.aborted) return;
        const { result, assist, location } = await this.translateLocal(id, record.original, sourceLang, targetLang, signal);
        if (signal.aborted || this.isTerminal || result.status === 'cancelled') return;
        if (location && location !== 'none') this.updateSegment(id, { processedOn: location });
        this.finishOutbound(id, seq, record.original, sourceLang, targetLang, result, finalAt, { ...(assist ? { assist } : {}), ...(this.privFlag() ? { priv: 'device' as const } : {}) });
      })
      .catch((err) => {
        this.diag.count('pipeline_errors');
        console.error('segment pipeline failed:', err);
      })
      .finally(() => {
        if (!this.disposed) this.commit({ translating: Math.max(0, this.snap.translating - 1) });
      });
  }

  /** Sends a translated local segment and updates its record (shared by both pipelines). */
  private finishOutbound(
    id: string,
    seq: number | undefined,
    original: string,
    sourceLang: string,
    targetLang: string,
    result: TranslationResult,
    finalAt: number,
    extra: Partial<Extract<PeerMessage, { t: 'segment' }>>
  ): void {
    const untranslated = result.status === 'failed';
    const latencyMs = Math.round(this.now() - finalAt);
    this.diag.sample('final_to_send_ms', latencyMs);
    this.diag.record('seg_translated', {
      segmentId: id,
      status: result.status,
      error: result.error?.code,
      attempts: result.attempts,
      ms: result.latencyMs,
    });
    this.call?.send({
      t: 'segment',
      id,
      seq,
      original,
      translated: result.text,
      sourceLang,
      targetLang,
      sentAt: this.now(),
      ...(untranslated ? { untranslated: true } : {}),
      ...extra,
    });
    this.diag.record('seg_sent', { segmentId: id });
    this.updateSegment(id, {
      translated: result.text,
      translation: untranslated ? 'failed' : result.status === 'passthrough' ? 'passthrough' : 'translated',
      untranslated: untranslated || undefined,
      latencyMs,
    });
    this.commit({ lastLatencyMs: latencyMs });
    this.emit({ type: 'translation.updated', segmentId: id, status: untranslated ? 'failed' : 'translated', error: result.error?.code });
    if (untranslated) {
      const message =
        result.error?.code === 'rate-limited'
          ? 'Too many requests — your words were sent untranslated. Try again shortly.'
          : 'Translation unavailable — your words were sent untranslated.';
      this.setNotice({ code: 'untranslated-out', message, sticky: false });
    }
  }

  // ==========================================================================
  // Phase 5 communication intelligence — controls
  // ==========================================================================

  /** Immediate off switch: context is cleared and translation is context-free from the next phrase. */
  setIntelligenceEnabled(on: boolean): void {
    if (!this.aci) return;
    this.aci.setPrefs({ enabled: on });
    this.diag.record('aci_prefs', { status: on ? 'on' : 'off' });
    this.refreshAci();
  }

  setIntelligencePrefs(patch: Partial<Omit<AciPrefs, 'enabled'>>): void {
    if (!this.aci) return;
    const clean: Partial<AciPrefs> = {};
    if (patch.mode === 'faithful' || patch.mode === 'concise') clean.mode = patch.mode;
    for (const k of ['assist', 'notes', 'suggestions'] as const) if (typeof patch[k] === 'boolean') clean[k] = patch[k];
    this.aci.setPrefs(clean);
    this.diag.record('aci_prefs', { mode: this.aci.getPrefs().mode });
    this.refreshAci();
  }

  /** Forget everything learned in this call so far. */
  resetIntelligenceContext(): void {
    if (!this.aci) return;
    this.aci.reset();
    this.diag.count('aci_context_reset');
    this.refreshAci();
  }

  /** User correction of the context summary: drop one item (not re-learned this call). */
  removeContextItem(id: string): void {
    if (this.aci?.removeEntity(id)) this.refreshAci();
  }

  /** Keep a name/term exactly as spoken in my translations this call. */
  pinContextTerm(text: string): void {
    if (this.aci?.pinTerm(text)) this.refreshAci();
  }

  dismissSuggestion(id: string): void {
    if (!this.aci) return;
    this.aci.dismiss(id);
    this.diag.count('suggestions_dismissed');
    this.refreshAci();
  }

  /** Phase 4 drafts (provisional translations before a phrase is complete): user can turn them off. */
  setDraftsEnabled(on: boolean): void {
    if (this.drafts === on) return;
    this.drafts = on;
    if (!on) this.abortProvisional();
    this.diag.record('drafts', { status: on ? 'on' : 'off' });
    this.commit({ conversation: { ...this.snap.conversation, drafts: on } });
  }

  // ==========================================================================
  // Phase 5 communication intelligence — pipeline hooks
  // ==========================================================================

  /**
   * Translates one of MY phrases, letting ACI propose a context-assisted MT
   * input. The original is always what is shown and sent as `original`; the
   * translator's raw input never leaks (failure → original; malformed
   * protected names → one context-free retry).
   */
  private async translateLocal(id: string, text: string, from: string, to: string, signal: AbortSignal): Promise<{ result: TranslationResult; assist?: string[]; location?: 'device' | 'cloud' | 'none' }> {
    if (this.policy) return this.translateLocalRouted(id, text, from, to, signal);
    const req = { segmentId: id, text, from, to, signal };
    const aci = this.aci;
    if (!aci) return { result: await this.deps.translate(req) };
    let d: OutboundDecision | null = null;
    const t0 = perfNow();
    try {
      d = aci.outbound({ segmentId: id, text, lang: from, targetLang: to });
    } catch (err) {
      this.diag.count('aci_errors');
      console.error('intelligence failed (faithful fallback):', err);
    }
    this.diag.sample('aci_ms', Math.round((perfNow() - t0) * 100) / 100);
    if (d) this.applyOutbound(d);
    const same = from.split('-')[0] === to.split('-')[0];
    if (!d || same || d.mtInput === text) return { result: await this.deps.translate(req) };
    const r = await this.deps.translate({ ...req, text: d.mtInput });
    if (r.status === 'cancelled') return { result: { ...r, text } };
    if (r.status === 'failed') return { result: { ...r, text } };
    const restored = aci.restoreTranslation(d, r.text);
    if (restored.ok) return { result: { ...r, text: restored.text }, assist: this.peerAci && d.assist.length ? d.assist : undefined };
    // Malformed provider output for protected names → context-free path (bounded: one retry).
    this.diag.count('aci_restore_fallback');
    this.updateSegment(id, { assisted: undefined, notes: [{ kind: 'assisted', text: 'Context assistance skipped for this phrase (translation changed protected names).' }] });
    if (signal.aborted) return { result: { ...r, status: 'cancelled', text } };
    return { result: await this.deps.translate(req) };
  }

  /** Phase 6 variant: same Phase 5 semantics, but every step goes through the processing policy. */
  private async translateLocalRouted(id: string, text: string, from: string, to: string, signal: AbortSignal): Promise<{ result: TranslationResult; assist?: string[]; location: 'device' | 'cloud' | 'none' }> {
    const req: TranslationRequest = { segmentId: id, text, from, to, signal };
    let d: OutboundDecision | null = null;
    if (this.aci) {
      const t0 = perfNow();
      try {
        d = this.aci.outbound({ segmentId: id, text, lang: from, targetLang: to });
      } catch (err) {
        this.diag.count('aci_errors');
        console.error('intelligence failed (faithful fallback):', err);
      }
      this.diag.sample('aci_ms', Math.round((perfNow() - t0) * 100) / 100);
      if (d) this.applyOutbound(d);
    }
    if (from.split('-')[0] === to.split('-')[0]) {
      return { result: { segmentId: id, status: 'passthrough', text: text.trim(), attempts: 0, latencyMs: 0 }, location: 'device' };
    }
    const cloudText = d && d.mtInput !== text ? d.mtInput : undefined;
    const r = await this.routedTranslate(req, { cloudText });
    if (r.location === 'cloud' && cloudText && d && this.aci && r.result.status === 'translated') {
      const restored = this.aci.restoreTranslation(d, r.result.text);
      if (restored.ok) return { result: { ...r.result, text: restored.text }, assist: this.peerAci && d.assist.length ? d.assist : undefined, location: 'cloud' };
      this.diag.count('aci_restore_fallback');
      this.updateSegment(id, { assisted: undefined, notes: [{ kind: 'assisted', text: 'Context assistance skipped for this phrase (translation changed protected names).' }] });
      if (signal.aborted) return { result: { ...r.result, status: 'cancelled', text }, location: 'none' };
      return this.routedTranslate(req, {});
    }
    return r;
  }

  /**
   * Executes a translation plan: on-device and/or cloud steps in the order the
   * policy allows. A cloud step re-checks permission at execution time and is
   * aborted if permission is revoked mid-flight (then the next allowed step
   * runs). Bounded: each step at most once; no loops.
   */
  private async routedTranslate(
    req: TranslationRequest,
    opts: { cloudText?: string; peerRequestedDevice?: boolean }
  ): Promise<{ result: TranslationResult; location: 'device' | 'cloud' | 'none' }> {
    const policy = this.policy!;
    const reg = this.registry!;
    const plan = policy.plan('translate', { deviceAvailable: reg.canTranslate(req.from, req.to), cloudAvailable: true, peerRequestedDevice: opts.peerRequestedDevice });
    const cancelled = (): { result: TranslationResult; location: 'none' } => ({ result: { segmentId: req.segmentId, status: 'cancelled', text: req.text, attempts: 0, latencyMs: 0 }, location: 'none' });
    let last: TranslationResult = { segmentId: req.segmentId, status: 'failed', text: req.text, error: { code: 'unavailable', message: plan.reason }, attempts: 0, latencyMs: 0 };
    let why: RouteReason | 'not-covered' = plan.reason;
    for (const step of plan.steps) {
      if (req.signal?.aborted) return cancelled();
      if (step === 'device') {
        const o = await reg.translate({ text: req.text, from: req.from, to: req.to, signal: req.signal });
        if (o.ok) {
          policy.noteDevice('translate', plan.reason);
          this.diag.record('route', { kind: 'translate', status: 'device', reason: plan.reason });
          return { result: { segmentId: req.segmentId, status: 'translated', text: o.text, attempts: 1, latencyMs: Math.round(o.ms) }, location: 'device' };
        }
        const miss = 'reason' in o ? o.reason : 'error';
        if (miss === 'cancelled') return cancelled();
        why = 'not-covered';
        this.diag.count(`local_${miss}`);
        continue;
      }
      let gate: { signal: AbortSignal; end: () => void };
      try {
        gate = policy.beginRemote('translate', { peerRequestedDevice: opts.peerRequestedDevice });
      } catch {
        continue; // permission changed since planning
      }
      try {
        const r = await this.deps.translate({ ...req, text: opts.cloudText ?? req.text, signal: anySignal(req.signal, gate.signal) });
        if (r.status === 'cancelled') {
          if (req.signal?.aborted) return cancelled();
          this.diag.count('remote_revoked');
          continue; // revoked mid-flight → next allowed step (never resumes remotely)
        }
        if (r.status === 'failed') {
          if (r.error?.code === 'network' || r.error?.code === 'timeout') this.netmon?.networkFailure();
          last = { ...r, text: req.text }; // never leak the MT input
          continue;
        }
        this.netmon?.success();
        this.diag.record('route', { kind: 'translate', status: 'cloud', reason: plan.reason });
        return { result: r, location: 'cloud' };
      } finally {
        gate.end();
      }
    }
    this.diag.record('route', { kind: 'translate', status: 'none', reason: why });
    if (last.error?.code === 'unavailable') last = { ...last, error: { code: 'unavailable', message: why } };
    return { result: last, location: 'none' };
  }

  // ==========================================================================
  // Phase 6 processing modes — controls and status
  // ==========================================================================

  /** User choice of processing mode. Changing it never resumes remote work that was revoked. */
  setProcessingMode(mode: ProcessingMode): void {
    const policy = this.policy;
    if (!policy || !PROCESSING_MODES.includes(mode) || mode === policy.mode || this.isTerminal) return;
    policy.setMode(mode);
    try {
      this.deps.storage?.setItem(PROCESSING_STORAGE_KEY, mode);
    } catch {
      /* setting lasts for this page */
    }
    this.deps.clearCaches?.();
    this.diag.record('processing', { mode });
    this.sendHello();
    this.refreshProcessing();
  }

  /**
   * "Clear what this call knows": cancels pending playback and remote work,
   * drops client caches, the routing audit and the intelligence context.
   * (The visible transcript stays until the call ends.)
   */
  clearPrivateData(): void {
    if (this.deps.playback.active) this.interrupt('stop');
    this.abortProvisional();
    this.deps.clearCaches?.();
    this.policy?.clearAudit();
    this.registry?.unloadAll();
    if (this.aci) {
      this.aci.reset();
      this.refreshAci();
    }
    this.diag.count('private_data_cleared');
  }

  /** Typed message: no audio, no speech recognition; translated under the current policy. */
  sendText(raw: string): void {
    const text = raw.replace(/\s+/g, ' ').trim().slice(0, 500);
    if (!text || this.isTerminal || this.phase.state !== 'connected') return;
    if (this.seg && this.outq) {
      const turn = newSegmentId();
      this.onUnitCommitted({ id: newSegmentId(), turn, text, rev: 0, repaired: false, uncertain: false }, 'complete');
      const last = this.ledger.list().at(-1);
      if (last?.speaker === 'local') this.updateSegment(last.id, { typed: true });
    } else {
      this.onLocalFinal({ text, lang: this.config.lang, startedAt: this.now(), endedAt: this.now(), engine: 'webspeech' });
      const last = this.ledger.list().at(-1);
      if (last?.speaker === 'local') this.updateSegment(last.id, { typed: true });
    }
    this.diag.count('typed_messages');
  }

  /** Our words should stay on-device on the other side too (sent to priv1 peers). */
  private privFlag(): boolean {
    return Boolean(this.policy && this.peerPrivCap && this.policy.mode === 'local-only');
  }

  private asrPlan(audit: boolean) {
    const policy = this.policy!;
    const ctx = {
      deviceAvailable: Boolean(this.registry?.hasLocalAsr(this.config.lang)),
      cloudAvailable: Boolean(this.snap.features.stt),
      cloudConsent: this.privacy.hasConsent(this.snap.features.route),
    };
    return audit ? policy.plan('asr', ctx) : policy.preview('asr', ctx);
  }

  /** Resolved when a segment STARTS playing (so later mode changes apply to queued speech). */
  private ttsRouteFor(lang: string, peerRequestedDevice: boolean): TtsRoute {
    const policy = this.policy;
    if (!policy) return 'cloud-first';
    const plan = policy.plan('tts', { deviceAvailable: Boolean(this.registry?.hasLocalVoice(lang)), cloudAvailable: true, peerRequestedDevice });
    this.diag.record('route', { kind: 'tts', status: plan.steps[0] ?? 'none', reason: plan.reason });
    if (!plan.steps.length || (plan.steps[0] === 'device' && plan.steps.length === 1)) return 'device-only';
    if (plan.steps[0] === 'device') return 'device-first';
    return 'cloud-first';
  }

  private onPolicyChange(): void {
    const policy = this.policy;
    if (!policy || this.isTerminal) return;
    const cloudBlocked = !policy.cloudPermitted('translate').ok;
    if (cloudBlocked) {
      // Speech that would be synthesized in the cloud must not continue.
      if (this.deps.playback.active) this.interrupt('stop');
      this.abortProvisional();
    }
    this.refreshProcessing();
  }

  /** Publishes the capability matrix for the current mode/network (no audit entries). */
  private refreshProcessing(): void {
    const policy = this.policy;
    const reg = this.registry;
    if (!policy || !reg || this.disposed) return;
    const peerLang = this.snap.peer.lang;
    const lang = this.config.lang;
    const row = (task: InferenceTask, label: CapabilityRow['task'], deviceAvailable: boolean, cloudAvailable: boolean, deviceDetail: string, cloudDetail: string): CapabilityRow => {
      const p = policy.preview(task, { deviceAvailable, cloudAvailable, cloudConsent: true });
      const has = (l: 'device' | 'cloud') => p.steps.includes(l);
      const state: CapabilityState = has('device') && has('cloud') ? 'device+cloud' : has('device') ? 'device' : has('cloud') ? 'cloud' : 'unavailable';
      const why: Record<string, string> = {
        'local-only': 'not available on this device in On-device mode',
        offline: 'needs a connection — you’re offline',
        unavailable: 'not available',
        'no-consent': 'needs your permission',
        'peer-requested-device': 'the other person asked for on-device processing',
      };
      const detail = state === 'unavailable' ? why[p.reason] ?? 'not available' : state === 'device' ? deviceDetail : state === 'cloud' ? cloudDetail : `${deviceDetail}; otherwise ${cloudDetail}`;
      return { task: label, state, detail };
    };
    const pairOk = peerLang ? reg.canTranslate(lang, peerLang) : false;
    const caps: CapabilityRow[] = [
      row('asr', 'speech-input', reg.hasLocalAsr(lang), Boolean(this.snap.features.stt), 'on this device', this.snap.features.route === 'server-stt' ? 'Talktual server → ElevenLabs' : 'your browser’s cloud speech service'),
      row('translate', 'translation', pairOk, true, 'on this device — common phrases only', `Talktual server → ${this.server?.translator ?? 'translation service'}`),
      row('caption', 'captions', pairOk, this.snap.features.liveCaptions, 'on this device — common phrases only', `Talktual server → ${this.server?.translator ?? 'translation service'}`),
      row('tts', 'speech-output', reg.hasLocalVoice(lang), true, 'on-device voice', this.server?.tts ? 'Talktual server → ElevenLabs (or a browser voice)' : 'browser voice (may be a cloud voice)'),
      { task: 'typed-input', state: 'device', detail: 'always available — no audio is processed' },
      ...(this.aci ? [{ task: 'assistance' as const, state: (this.aci.getPrefs().enabled ? 'device' : 'unavailable') as CapabilityState, detail: this.aci.getPrefs().enabled ? 'on this device' : 'turned off' }] : []),
    ];
    const degraded = caps.some((c) => c.state === 'unavailable' && c.task !== 'captions' && c.task !== 'assistance');
    this.commit({ processing: { ...this.snap.processing, mode: policy.mode, network: policy.network, capabilities: caps, degraded, runtimes: reg.infos(), peerPrivate: this.snap.processing.peerPrivate } });
  }

  private applyOutbound(d: OutboundDecision): void {
    const notes = [...d.notes];
    if (d.correction) notes.unshift({ kind: 'correction', text: `Corrects earlier: ${d.correction.from} → ${d.correction.to}` });
    const assisted = d.protections.length > 0 || d.edits.length > 0;
    this.updateSegment(d.segmentId, {
      ...(notes.length ? { notes: notes.slice(0, 3) } : {}),
      ...(d.signals.length ? { signals: d.signals.filter((s) => s.confidence >= 0.5) } : {}),
      ...(assisted ? { assisted: true } : {}),
    });
    if (d.correction) this.updateSegment(d.correction.segmentId, { correctedBy: d.segmentId });
    this.recordAci(d.segmentId, 'outbound', d.fallback, assisted, d.notes.length, d.suggestions.length, d.signals);
    this.refreshAci();
  }

  private aciInbound(
    id: string,
    original: string,
    sourceLang: string,
    translated: string,
    untranslated: boolean,
    assist: string[] | undefined
  ): { record: Partial<SegmentRecord>; speak?: string; correctsId?: string } {
    const aci = this.aci;
    if (!aci) return { record: {} };
    const t0 = perfNow();
    try {
      const d = aci.inbound({ segmentId: id, original, sourceLang, translated, listenerLang: this.config.lang, assist: this.peerAci ? assist : undefined });
      this.diag.sample('aci_ms', Math.round((perfNow() - t0) * 100) / 100);
      this.recordAci(id, 'inbound', d.fallback, Boolean(assist?.length), d.notes.length, d.suggestions.length, d.signals);
      this.refreshAci();
      return {
        record: {
          ...(d.notes.length ? { notes: d.notes } : {}),
          ...(d.signals.length ? { signals: d.signals.filter((s) => s.confidence >= 0.5) } : {}),
          ...(this.peerAci && assist?.length ? { assisted: true } : {}),
        },
        // Concise applies to translated speech in MY language only.
        speak: !untranslated && d.speak ? d.speak.text : undefined,
        correctsId: d.correction?.segmentId,
      };
    } catch (err) {
      this.diag.count('aci_errors');
      console.error('intelligence failed (faithful fallback):', err);
      return { record: {} };
    }
  }

  /** Diagnostics: categories, counts and timings only — never text. */
  private recordAci(segmentId: string, kind: 'inbound' | 'outbound', fallback: string | undefined, assisted: boolean, notes: number, suggestions: number, signals: { category: string; confidence: number }[]): void {
    this.diag.record('aci', { segmentId, kind, ...(fallback ? { reason: fallback } : {}), assisted, notes, suggestions });
    if (fallback) this.diag.count(`aci_fallback_${fallback}`);
    for (const s of signals) {
      if (s.confidence < 0.5) continue;
      this.diag.count(`signal_${s.category}`);
      this.diag.record('aci_signal', { segmentId, kind: s.category, confidence: s.confidence });
    }
  }

  /** Publishes prefs/suggestions/context summary; re-runs when the next suggestion expires. */
  private refreshAci(): void {
    const aci = this.aci;
    if (!aci || this.disposed) return;
    const suggestions = aci.activeSuggestions();
    this.commit({ intelligence: { ...this.snap.intelligence, prefs: aci.getPrefs(), suggestions, context: aci.summary() } });
    this.refreshProcessing();
    if (this.aciTimer) this.clearT(this.aciTimer);
    this.aciTimer = null;
    if (suggestions.length && !this.isTerminal) {
      const next = Math.min(...suggestions.map((s) => s.expiresAt)) - this.now();
      this.aciTimer = this.setT(() => {
        this.aciTimer = null;
        this.refreshAci();
      }, Math.max(0, next) + 5);
    }
  }

  // ==========================================================================
  // Phase 4 conversation layer — local speaker side
  // ==========================================================================

  /** Starts a speaking turn (press, or first speech in hands-free mode). */
  private openFloor(): void {
    if (!this.seg || this.floorOpen) return;
    this.turnId = newSegmentId();
    this.floorOpen = true;
    this.seg.beginTurn(this.turnId);
    if (this.peerCaps) this.call?.send({ t: 'floor', turn: this.turnId, open: true });
  }

  private closeFloor(): void {
    if (this.floorTimer) this.clearT(this.floorTimer);
    this.floorTimer = null;
    if (!this.floorOpen || !this.turnId) return;
    this.floorOpen = false;
    if (this.peerCaps) this.call?.send({ t: 'floor', turn: this.turnId, open: false });
    this.syncTalking();
  }

  /** Hands-free: speech opens the floor; silence for floorIdleMs closes it. */
  private touchContinuousFloor(): void {
    if (!this.seg || !this.conv || this.privacy.mode !== 'continuous') return;
    if (!this.floorOpen) {
      this.openFloor();
      this.syncTalking();
    }
    if (this.floorTimer) this.clearT(this.floorTimer);
    const arm = () => {
      this.floorTimer = this.setT(() => {
        this.floorTimer = null;
        if (this.seg?.current) arm(); // a held unit is still deciding — bounded by maxOpenMs
        else this.closeFloor();
      }, this.conv!.floorIdleMs);
    };
    arm();
  }

  private onUnitHeld(u: Unit, kind: 'held' | 'revised'): void {
    this.diag.record('conv_unit', { segmentId: u.id, state: kind, ms: this.seg?.holdMs, rev: u.rev });
    this.emit({ type: 'conversation.unit', segmentId: u.id, state: kind });
    // A revision makes any in-flight provisional translation stale.
    if (kind === 'revised' && this.prov?.id === u.id) this.abortProvisional();
    this.commit({ conversation: { ...this.snap.conversation, held: { id: u.id, text: u.text, provisional: this.provSent.has(u.id) } } });
    this.maybeSendProvisional(u);
  }

  private maybeSendProvisional(u: Unit): void {
    if (!this.conv || !this.drafts || !this.peerCaps || !this.call || this.lat?.degraded || this.prov) return;
    const now = this.now();
    if (now - this.lastProvAt < this.conv.provisionalMinIntervalMs) return;
    this.lastProvAt = now;
    const controller = new AbortController();
    const onWorkAbort = () => controller.abort();
    this.work.signal.addEventListener('abort', onWorkAbort, { once: true });
    this.prov = { id: u.id, controller };
    const sourceLang = this.config.lang;
    const targetLang = this.snap.peer.lang ?? sourceLang;
    const draftReq = { segmentId: u.id, text: u.text, from: sourceLang, to: targetLang, signal: controller.signal };
    (this.policy ? this.routedTranslate(draftReq, {}).then((x) => x.result) : this.deps.translate(draftReq))
      .then((r) => {
        if (controller.signal.aborted || this.isTerminal || r.status === 'cancelled' || r.status === 'failed') return;
        // Only if the unit is still held at the same revision: never send a provisional after its final.
        const cur = this.seg?.current;
        if (!cur || cur.unit.id !== u.id || cur.unit.rev !== u.rev) return;
        this.call?.send({
          t: 'segment',
          id: u.id,
          original: u.text,
          translated: r.text,
          sourceLang,
          targetLang,
          sentAt: this.now(),
          state: 'provisional',
          rev: u.rev,
          turn: u.turn,
          ...(u.uncertain ? { uncertain: true } : {}),
        });
        this.provSent.add(u.id);
        this.seg?.markProvisional(u.id);
        this.diag.count('provisional_sent');
        if (this.snap.conversation.held?.id === u.id) {
          this.commit({ conversation: { ...this.snap.conversation, held: { ...this.snap.conversation.held, provisional: true } } });
        }
      })
      .catch(() => {})
      .finally(() => {
        this.work.signal.removeEventListener('abort', onWorkAbort);
        if (this.prov?.controller === controller) this.prov = null;
      });
  }

  private abortProvisional(): void {
    this.prov?.controller.abort();
    this.prov = null;
  }

  private onUnitDiscarded(u: Unit): void {
    if (this.prov?.id === u.id) this.abortProvisional();
    if (this.provSent.delete(u.id)) {
      this.call?.send({ t: 'retract', id: u.id });
      this.diag.count('provisional_retracted');
    }
    this.diag.record('conv_unit', { segmentId: u.id, state: 'discarded' });
    this.emit({ type: 'conversation.unit', segmentId: u.id, state: 'discarded' });
    if (this.snap.conversation.held) this.commit({ conversation: { ...this.snap.conversation, held: null } });
  }

  private onUnitCommitted(u: Unit, reason: CommitReason): void {
    if (this.prov?.id === u.id) this.abortProvisional();
    this.provSent.delete(u.id);
    this.diag.record('conv_unit', { segmentId: u.id, state: 'final', reason, rev: u.rev, repaired: u.repaired, uncertain: u.uncertain });
    this.diag.record('seg_final', { segmentId: u.id });
    this.diag.count(`commit_${reason}`);
    this.emit({ type: 'conversation.unit', segmentId: u.id, state: 'final', reason });
    if (this.isTerminal || !this.outq) return;
    const record: SegmentRecord = {
      id: u.id,
      seq: this.ledger.nextSeq(),
      speaker: 'local',
      speakerName: this.config.userName,
      original: u.text,
      translated: '',
      sourceLang: this.config.lang,
      targetLang: this.snap.peer.lang ?? this.config.lang,
      translation: 'pending',
      createdAt: this.now(),
      state: 'final',
      turn: u.turn,
      uncertain: u.uncertain || undefined,
      repaired: u.repaired || undefined,
    };
    const { entry, merged } = this.outq.push(u, this.now());
    if (merged) {
      // Backpressure: folded into the unit already waiting for translation.
      this.diag.count('units_merged');
      this.updateSegment(entry.unit.id, { original: entry.unit.text, repaired: entry.unit.repaired || undefined, uncertain: entry.unit.uncertain || undefined });
      this.commit({ conversation: { ...this.snap.conversation, held: null } });
      if (!this.snap.conversation.degraded) {
        this.setNotice({ code: 'slow', message: 'Translation is slow — combining phrases to keep up.', sticky: false });
      }
      return;
    }
    this.addSegment(record);
    this.commit({ translating: this.snap.translating + 1, conversation: { ...this.snap.conversation, held: null } });
    this.diag.sample('outbound_depth', this.outq.depth);
    this.pumpOutbound();
  }

  /** One translation in flight; the next waiting unit (possibly merged) follows. */
  private pumpOutbound(): void {
    const q = this.outq;
    if (!q || q.inFlight) return;
    const signal = this.work.signal;
    if (signal.aborted || this.isTerminal) {
      q.clear();
      return;
    }
    const p = q.take();
    if (!p) return;
    const { unit } = p;
    const rec = this.ledger.get(unit.id);
    const seq = rec?.seq;
    // The language the words were spoken in (the user may have switched since).
    const sourceLang = rec?.sourceLang ?? this.config.lang;
    const targetLang = this.snap.peer.lang ?? sourceLang;
    const started = this.now();
    this.translateLocal(unit.id, unit.text, sourceLang, targetLang, signal)
      .then(({ result, assist, location }) => {
        if (signal.aborted || this.isTerminal || result.status === 'cancelled') return;
        if (location && location !== 'none') this.updateSegment(unit.id, { processedOn: location });
        if (this.lat?.observe(this.now() - started)) {
          const degraded = this.lat.degraded;
          this.diag.record('conv_degraded', { status: degraded ? 'on' : 'off' });
          this.commit({ conversation: { ...this.snap.conversation, degraded } });
          if (degraded) this.setNotice({ code: 'slow', message: 'Translation is slow — combining phrases to keep up.', sticky: false });
        }
        const extra = {
          ...(this.peerCaps
            ? { state: 'final' as const, rev: unit.rev, turn: unit.turn, ...(unit.uncertain ? { uncertain: true } : {}), ...(unit.repaired ? { repaired: true } : {}) }
            : {}),
          ...(assist ? { assist } : {}),
          ...(this.privFlag() ? { priv: 'device' as const } : {}),
        };
        this.finishOutbound(unit.id, seq, unit.text, sourceLang, targetLang, result, p.committedAt, extra);
      })
      .catch((err) => {
        this.diag.count('pipeline_errors');
        console.error('segment pipeline failed:', err);
      })
      .finally(() => {
        q.done();
        if (this.disposed) return;
        this.commit({ translating: Math.max(0, this.snap.translating - 1) });
        this.pumpOutbound();
      });
  }

  // ==========================================================================
  // Phase 4 conversation layer — listener side
  // ==========================================================================

  private isLocalTalking(): boolean {
    return this.snap.holding || (this.privacy.mode === 'continuous' && this.floorOpen);
  }

  /** When the local user stops talking, play what arrived meanwhile (age-bounded). */
  private syncTalking(): void {
    if (!this.listen) return;
    const talking = this.isLocalTalking();
    if (this.wasTalking && !talking) {
      for (const h of this.listen.releaseHeld()) this.enqueuePeer(h.item, h.turn);
      if (this.snap.conversation.overlapHeld) this.commit({ conversation: { ...this.snap.conversation, overlapHeld: 0 } });
    }
    this.wasTalking = talking;
  }

  private enqueuePeer(item: PlaybackItem, turn: string | undefined): void {
    this.listen?.noteActive(item.segmentId, turn);
    this.deps.playback.enqueue(item);
  }

  /** Marks records of these turns as interrupted (UI + transcript; never deletes them). */
  private markTurns(speaker: 'local' | 'peer', turns: string[]): void {
    const set = new Set(turns);
    let changed = false;
    for (const r of this.ledger.list()) {
      if (r.speaker === speaker && r.turn && set.has(r.turn) && !r.interrupted && r.playback !== 'ended') {
        this.ledger.update(r.id, { interrupted: true });
        changed = true;
      }
    }
    if (changed) this.commit({ segments: this.ledger.list() });
  }

  /** Phase 4 peer messages. Returns true if handled. */
  private onConversationMessage(msg: PeerMessage): boolean {
    const now = this.now();
    if (msg.t === 'floor') {
      if (!this.listen) return true;
      this.listen.onPeerFloor(msg.turn, msg.open, now);
      if (this.snap.conversation.peerTalking !== msg.open) this.commit({ conversation: { ...this.snap.conversation, peerTalking: msg.open } });
      return true;
    }
    if (msg.t === 'interrupt') {
      if (!this.listen) return true;
      this.diag.count('interrupted_by_peer');
      this.markTurns('local', [msg.turn]);
      if (this.aci) {
        for (const r of this.ledger.list()) if (r.speaker === 'local' && r.turn === msg.turn) this.aci.dropSegment(r.id);
        this.refreshAci();
      }
      return true;
    }
    if (msg.t === 'retract') {
      const r = this.ledger.get(msg.id);
      if (r && r.speaker === 'peer' && r.state === 'provisional') {
        this.ledger.remove(msg.id);
        this.commit({ segments: this.ledger.list() });
        this.diag.count('provisional_retracted_in');
      }
      return true;
    }
    if (msg.t === 'segment' && msg.state === 'provisional') {
      if (!this.listen) {
        this.diag.count('provisional_ignored');
        return true;
      }
      this.listen.touchPeerTurn(msg.turn, now);
      // Never let a provisional replace a final, or an older revision a newer one.
      const existing = this.ledger.get(msg.id);
      if (this.ledger.hasSeenPeer(msg.id) || (existing && (existing.rev ?? -1) >= (msg.rev ?? 0))) {
        this.diag.count('provisional_stale_dropped');
        return true;
      }
      const myLang = this.config.lang;
      const usable = msg.targetLang.split('-')[0] === myLang.split('-')[0];
      this.ledger.upsert({
        id: msg.id,
        speaker: 'peer',
        speakerName: this.snap.peer.name ?? 'Them',
        original: msg.original,
        translated: usable ? msg.translated : '',
        sourceLang: msg.sourceLang,
        targetLang: myLang,
        translation: usable ? 'translated' : 'pending',
        createdAt: existing?.createdAt ?? now,
        state: 'provisional',
        rev: msg.rev ?? 0,
        turn: msg.turn,
        uncertain: msg.uncertain || undefined,
      });
      this.commit({ segments: this.ledger.list() });
      this.diag.count('provisional_received');
      return true;
    }
    return false;
  }

  // ==========================================================================
  // Incoming pipeline
  // ==========================================================================

  private onPeerMessage(msg: PeerMessage): void {
    if (this.isTerminal) return;
    const myLang = this.config.lang;

    if (msg.t === 'voice') {
      if (this.voice) this.voice.onPeerMessage(msg);
      else this.diag.count('voice_messages_ignored');
      return;
    }

    if (this.onConversationMessage(msg)) return;

    if (msg.t === 'hello') {
      this.peerCaps = Boolean(this.conv && Array.isArray(msg.caps) && msg.caps.includes(CONVERSATION_CAP));
      this.peerAci = Boolean(this.aci && Array.isArray(msg.caps) && msg.caps.includes(ACI_CAP));
      this.peerPrivCap = Boolean(this.policy && Array.isArray(msg.caps) && msg.caps.includes(PRIV_CAP));
      this.aci?.setLangs(this.config.lang, msg.lang);
      if (this.aci && this.snap.intelligence.peerCapable !== this.peerAci) this.commit({ intelligence: { ...this.snap.intelligence, peerCapable: this.peerAci } });
      if (this.snap.conversation.peerCapable !== this.peerCaps) {
        this.commit({ conversation: { ...this.snap.conversation, peerCapable: this.peerCaps } });
      }
      this.commit({ peer: { name: msg.name || null, lang: msg.lang } });
      this.refreshProcessing();
      this.emit({ type: 'peer.updated', name: msg.name || null, lang: msg.lang });
      return;
    }

    if (msg.t === 'partial') {
      this.listen?.touchPeerTurn(msg.turn, this.now());
      if (!msg.text) {
        this.commit({ partial: { ...this.snap.partial, peer: '' } });
        return;
      }
      // Without a fast translator, showing raw source text is more honest than
      // burning a rate-limited quota on text that changes every 200ms.
      if (!this.snap.features.liveCaptions) {
        this.commit({ partial: { ...this.snap.partial, peer: msg.text } });
        this.emit({ type: 'transcript.partial', speaker: 'peer', text: msg.text, lang: msg.lang });
        return;
      }
      if (this.partialTimer) this.clearT(this.partialTimer);
      if (this.policy && this.registry) {
        const policy = this.policy;
        const reg = this.registry;
        const peerPriv = this.peerPrivCap && msg.priv === 'device';
        const plan = policy.preview('caption', { deviceAvailable: reg.canTranslate(msg.lang, myLang), cloudAvailable: true, peerRequestedDevice: peerPriv });
        if (plan.steps[0] !== 'cloud') {
          // On-device (or nothing): a covered phrase is translated locally, otherwise the raw caption is shown.
          if (this.partialTimer) this.clearT(this.partialTimer);
          this.partialTimer = this.setT(() => {
            this.partialAbort?.abort();
            const controller = new AbortController();
            this.partialAbort = controller;
            const show = (t: string) => !controller.signal.aborted && !this.isTerminal && this.commit({ partial: { ...this.snap.partial, peer: t } });
            if (!plan.steps.length) return void show(msg.text);
            reg
              .translate({ text: msg.text, from: msg.lang, to: myLang, signal: controller.signal })
              .then((o) => show(o.ok ? o.text : msg.text))
              .catch(() => show(msg.text));
          }, PARTIAL_TRANSLATE_DEBOUNCE_MS);
          return;
        }
      }
      this.partialTimer = this.setT(() => {
        this.partialAbort?.abort();
        const controller = new AbortController();
        this.partialAbort = controller;
        let gate: { signal: AbortSignal; end: () => void } | null = null;
        if (this.policy) {
          try {
            gate = this.policy.beginRemote('caption', { peerRequestedDevice: this.peerPrivCap && msg.priv === 'device' });
          } catch {
            this.commit({ partial: { ...this.snap.partial, peer: msg.text } });
            return;
          }
        }
        const sig = gate ? anySignal(controller.signal, gate.signal) : controller.signal;
        this.deps
          .translateCaption(msg.text, msg.lang, myLang, sig)
          .then((t) => {
            if (!controller.signal.aborted && !this.isTerminal) this.commit({ partial: { ...this.snap.partial, peer: t } });
          })
          .catch((err) => {
            // Superseded: ignore. Real failure: show the raw interim text.
            if (!controller.signal.aborted && !this.isTerminal && (err as Error)?.name !== 'AbortError') {
              this.commit({ partial: { ...this.snap.partial, peer: msg.text } });
            }
          })
          .finally(() => gate?.end());
      }, PARTIAL_TRANSLATE_DEBOUNCE_MS);
      return;
    }

    // --- segment (final) ---
    if (msg.t !== 'segment') return;
    if (!this.ledger.acceptPeerSegment(msg.id)) {
      this.diag.count('duplicate_segments_dropped');
      this.diag.record('seg_duplicate', { segmentId: msg.id });
      return;
    }
    const receivedAt = this.now();
    this.diag.record('seg_received', { segmentId: msg.id });
    if (this.partialTimer) this.clearT(this.partialTimer);
    this.partialAbort?.abort();
    this.commit({ partial: { ...this.snap.partial, peer: '' } });

    const base: SegmentRecord = {
      id: msg.id,
      seq: msg.seq,
      speaker: 'peer',
      speakerName: this.snap.peer.name ?? 'Them',
      original: msg.original,
      translated: '',
      sourceLang: msg.sourceLang,
      targetLang: myLang,
      translation: 'pending',
      createdAt: receivedAt,
      ...(this.conv
        ? {
            state: 'final' as const,
            turn: msg.turn,
            rev: msg.rev,
            uncertain: msg.uncertain || undefined,
            repaired: msg.repaired || undefined,
          }
        : {}),
    };
    this.listen?.touchPeerTurn(msg.turn, receivedAt);
    // Phase 6: the sender asked that their words be processed on-device only.
    const peerPriv = Boolean(this.policy && this.peerPrivCap && msg.priv === 'device');
    if (peerPriv !== this.snap.processing.peerPrivate && this.policy) this.commit({ processing: { ...this.snap.processing, peerPrivate: peerPriv } });

    const deliver = (text: string, untranslated: boolean) => {
      if (this.isTerminal) return;
      // Phase 4: the listener cut into this turn — keep it readable, don't speak it.
      const interrupted = Boolean(this.listen?.isInterrupted(msg.turn));
      const held = !interrupted && Boolean(this.listen) && this.isLocalTalking();
      // Phase 5: notes/signals/concise delivery for what we hear (advisory, validated).
      const aci = this.aciInbound(msg.id, msg.original, msg.sourceLang, text, untranslated, msg.assist);
      const record: SegmentRecord = {
        ...base,
        translated: text,
        translation: untranslated ? 'failed' : 'translated',
        untranslated: untranslated || undefined,
        ...(interrupted ? { interrupted: true } : {}),
        ...(held ? { overlapHeld: true } : {}),
        ...aci.record,
        ...(peerPriv ? { privateRequest: true } : {}),
      };
      this.addSegment(record);
      if (aci.correctsId) this.updateSegment(aci.correctsId, { correctedBy: msg.id });
      if (interrupted) {
        this.diag.count('interrupted_suppressed');
        this.diag.record('conv_suppressed', { segmentId: msg.id, reason: 'interrupted' });
        return;
      }
      if (untranslated) {
        this.setNotice({ code: 'untranslated-in', message: 'Translation unavailable — playing their original words.', sticky: false });
      }
      // Untranslated text is spoken with a voice for its own language;
      // reading it with ours would mangle every word.
      this.receivedAt.set(msg.id, receivedAt);
      const item: PlaybackItem = {
        segmentId: msg.id,
        // Concise mode (my choice): fillers not spoken; the transcript keeps everything.
        text: aci.speak ?? text,
        lang: untranslated ? msg.sourceLang : myLang,
        // Resolved at playback start → a revocation applies to queued segments too.
        voice: this.voice ? () => this.voice?.peerHints() : undefined,
        ...(this.conv ? { notAfter: receivedAt + this.conv.maxPlaybackAgeMs } : {}),
        ...(this.policy ? { ttsRoute: () => this.ttsRouteFor(untranslated ? msg.sourceLang : myLang, peerPriv) } : {}),
      };
      if (held && this.listen) {
        // Overlap: don't talk over the local user; play when they stop (age-bounded).
        const evicted = this.listen.hold({ item, turn: msg.turn });
        if (evicted) {
          this.updateSegment(evicted.item.segmentId, { playback: 'dropped' });
          this.diag.count('playback_dropped');
        }
        this.diag.count('overlap_held');
        this.commit({ conversation: { ...this.snap.conversation, overlapHeld: this.listen.heldCount } });
        return;
      }
      this.enqueuePeer(item, msg.turn);
    };

    const sameLanguage = msg.sourceLang.split('-')[0] === myLang.split('-')[0];
    // The sender normally translates into our language using the handshake.
    // If their hello hadn't landed yet, we just switched language, or their
    // translation failed, fix it up on this side.
    if (!msg.untranslated && msg.targetLang.split('-')[0] === myLang.split('-')[0]) {
      deliver(msg.translated, false);
    } else if (sameLanguage) {
      deliver(msg.original, false);
    } else {
      const fixReq = { segmentId: msg.id, text: msg.original, from: msg.sourceLang, to: myLang, signal: this.work.signal };
      (this.policy ? this.routedTranslate(fixReq, { peerRequestedDevice: peerPriv }).then((x) => x.result) : this.deps.translate(fixReq))
        .then((r) => {
          if (r.status === 'cancelled') return;
          deliver(r.status === 'failed' ? msg.original : r.text, r.status === 'failed');
        })
        .catch(() => deliver(msg.original, true));
    }
  }

  private receivedAt = new Map<string, number>();

  private onPlaybackStatus(segmentId: string, status: SegmentRecord['playback'] & string, engine?: 'server' | 'browser'): void {
    if (this.disposed) return;
    this.updateSegment(segmentId, { playback: status });
    this.emit({ type: 'playback.status', segmentId, status, engine });
    this.diag.record('playback', { segmentId, status, engine });
    if (status === 'started') {
      const r = this.receivedAt.get(segmentId);
      if (r !== undefined) this.diag.sample('receive_to_playback_ms', this.now() - r);
    }
    if (status !== 'queued' && status !== 'started') {
      this.receivedAt.delete(segmentId);
      this.listen?.noteDone(segmentId);
    }
    if (status === 'dropped') this.diag.count('playback_dropped');

    const playing = this.deps.playback.active;
    if (playing !== this.snap.playing) this.commit({ playing });

    // Continuous mode is half-duplex: don't transcribe our own speaker.
    if (this.privacy.mode === 'continuous' && !this.isTerminal) {
      if (playing && this.asr?.active && !this.pausedForPlayback) {
        this.pausedForPlayback = true;
        this.asr.stop();
        this.activity.to('paused', 'playback');
        this.commitActivity();
        this.emitPrivacy();
      } else if (!playing && this.pausedForPlayback) {
        this.pausedForPlayback = false;
        if (this.micEnabled && this.phase.state === 'connected') {
          if (this.activity.state === 'paused') this.activity.to('ready', 'playback-done');
          this.startAsr('continuous');
        }
      }
    }
  }

  private interrupt(kind: 'barge-in' | 'stop', interruptedTurns?: number): void {
    const cancelled = this.deps.playback.cancelAll();
    this.commit({ playing: false });
    this.emit({ type: 'interruption', kind, cancelledSegments: cancelled, ...(interruptedTurns ? { interruptedTurns } : {}) });
    this.diag.record('interruption', { kind, n: cancelled });
  }

  // ==========================================================================
  // Helpers
  // ==========================================================================

  private sendHello(): void {
    const caps = [...(this.conv ? [CONVERSATION_CAP] : []), ...(this.aci ? [ACI_CAP] : []), ...(this.policy ? [PRIV_CAP] : [])];
    this.call?.send({ t: 'hello', lang: this.config.lang, name: this.config.userName, ...(caps.length ? { caps } : {}) });
  }

  private refreshFeatures(): void {
    const features = resolveFeatures(this.deps.browser, this.server);
    const route = features.route;
    const granted = this.privacy.hasConsent(route);
    this.commit({
      features,
      consent: {
        ...this.snap.consent,
        route,
        granted,
        disclosure: route ? disclosureFor(route, this.server?.translator ?? null, { drafts: Boolean(this.conv), intelligence: Boolean(this.aci) }) : null,
      },
    });
    this.refreshProcessing();
    const degraded = degradationMessage(features);
    if (degraded && features.serverKnown) this.setNotice({ code: 'degraded', message: degraded, sticky: true });
    if (granted && this.activity.state === 'idle') {
      this.activity.to('ready', 'consent-known');
      this.commitActivity();
    }
    // First time on this device and able to talk: ask up front, once.
    if (this.phase.state === 'connected' && features.canTalk && !granted && !this.consentDeclined && features.serverKnown) {
      this.promptConsent();
    }
  }

  private promptConsent(): void {
    const route = this.snap.features.route;
    if (!route || this.isTerminal) return;
    this.commit({
      consent: { route, granted: this.privacy.hasConsent(route), prompt: true, disclosure: disclosureFor(route, this.server?.translator ?? null, { drafts: Boolean(this.conv), intelligence: Boolean(this.aci) }) },
    });
  }

  private setPermission(state: PermissionState): void {
    if (state === this.snap.permission) return;
    this.commit({ permission: state });
    this.emit({ type: 'permission.changed', state });
    this.diag.record('permission', { state });
  }

  private addSegment(record: SegmentRecord): void {
    this.ledger.upsert(record);
    this.commit({ segments: this.ledger.list() });
    this.emit({ type: 'transcript.final', segment: record });
  }

  private updateSegment(id: string, patch: Partial<SegmentRecord>): void {
    if (this.ledger.update(id, patch)) this.commit({ segments: this.ledger.list() });
  }

  private setNotice(notice: Notice): void {
    if (this.noticeTimer) this.clearT(this.noticeTimer);
    this.noticeTimer = null;
    // A sticky (state) notice isn't replaced by a transient one.
    if (this.snap.notice?.sticky && !notice.sticky && this.snap.notice.code !== notice.code) {
      return;
    }
    this.commit({ notice });
    if (!notice.sticky) this.noticeTimer = this.setT(() => this.clearNotice(), NOTICE_MS);
  }

  private clearNotice(): void {
    if (this.noticeTimer) this.clearT(this.noticeTimer);
    this.noticeTimer = null;
    if (this.snap.notice) this.commit({ notice: null });
  }

  private emitPrivacy(): void {
    const capturing = Boolean(this.asr?.active);
    if (capturing !== this.snap.capturing) this.commit({ capturing });
    this.emit({
      type: 'privacy.changed',
      mode: this.privacy.mode,
      route: this.snap.features.route,
      consent: this.privacy.hasConsent(this.snap.features.route),
      capturing,
    });
  }

  private commitActivity(): void {
    if (this.snap.activity !== this.activity.state) this.commit({ activity: this.activity.state });
  }

  private emit(e: Parameters<typeof makeEvent>[0]): void {
    if (this.disposed) return;
    this.events.emit(makeEvent(e, this.now()) as EngineEvent);
  }

  private commit(patch: Partial<SessionSnapshot>): void {
    if (this.disposed) return;
    this.snap = { ...this.snap, ...patch };
    for (const fn of this.subscribers) {
      try {
        fn();
      } catch (err) {
        console.error('session subscriber failed:', err);
      }
    }
  }
}
