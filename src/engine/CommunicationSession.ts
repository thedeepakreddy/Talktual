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
import type { PlaybackItem, TranslationResult } from './ports';

export const PARTIAL_SEND_INTERVAL_MS = 180;
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
  now?: () => number;
  setTimeout?: typeof setTimeout;
  clearTimeout?: typeof clearTimeout;
}

export interface Notice {
  code: ErrorCode | 'info' | 'untranslated-out' | 'untranslated-in' | 'peer-away' | 'degraded' | 'slow';
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
      conversation: { enabled: Boolean(deps.conversation), peerCapable: false, degraded: false, held: null, overlapHeld: 0, peerTalking: false },
    };
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
      });
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
    this.call?.send({ t: 'partial', text, lang: this.config.lang, ...(this.peerCaps && this.turnId ? { turn: this.turnId } : {}) });
  }

  private onLocalFinal(seg: RecognizedSegment): void {
    if (this.isTerminal || !seg.text.trim()) return;
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
        const result = await this.deps.translate({ segmentId: id, text: record.original, from: sourceLang, to: targetLang, signal });
        if (signal.aborted || this.isTerminal || result.status === 'cancelled') return;
        this.finishOutbound(id, seq, record.original, sourceLang, targetLang, result, finalAt, {});
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
    if (!this.conv || !this.peerCaps || !this.call || this.lat?.degraded || this.prov) return;
    const now = this.now();
    if (now - this.lastProvAt < this.conv.provisionalMinIntervalMs) return;
    this.lastProvAt = now;
    const controller = new AbortController();
    const onWorkAbort = () => controller.abort();
    this.work.signal.addEventListener('abort', onWorkAbort, { once: true });
    this.prov = { id: u.id, controller };
    const sourceLang = this.config.lang;
    const targetLang = this.snap.peer.lang ?? sourceLang;
    this.deps
      .translate({ segmentId: u.id, text: u.text, from: sourceLang, to: targetLang, signal: controller.signal })
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
    this.deps
      .translate({ segmentId: unit.id, text: unit.text, from: sourceLang, to: targetLang, signal })
      .then((result) => {
        if (signal.aborted || this.isTerminal || result.status === 'cancelled') return;
        if (this.lat?.observe(this.now() - started)) {
          const degraded = this.lat.degraded;
          this.diag.record('conv_degraded', { status: degraded ? 'on' : 'off' });
          this.commit({ conversation: { ...this.snap.conversation, degraded } });
          if (degraded) this.setNotice({ code: 'slow', message: 'Translation is slow — combining phrases to keep up.', sticky: false });
        }
        const extra = this.peerCaps
          ? { state: 'final' as const, rev: unit.rev, turn: unit.turn, ...(unit.uncertain ? { uncertain: true } : {}), ...(unit.repaired ? { repaired: true } : {}) }
          : {};
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
      if (this.snap.conversation.peerCapable !== this.peerCaps) {
        this.commit({ conversation: { ...this.snap.conversation, peerCapable: this.peerCaps } });
      }
      this.commit({ peer: { name: msg.name || null, lang: msg.lang } });
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
      this.partialTimer = this.setT(() => {
        this.partialAbort?.abort();
        const controller = new AbortController();
        this.partialAbort = controller;
        this.deps
          .translateCaption(msg.text, msg.lang, myLang, controller.signal)
          .then((t) => {
            if (!controller.signal.aborted && !this.isTerminal) this.commit({ partial: { ...this.snap.partial, peer: t } });
          })
          .catch((err) => {
            // Superseded: ignore. Real failure: show the raw interim text.
            if (!controller.signal.aborted && !this.isTerminal && (err as Error)?.name !== 'AbortError') {
              this.commit({ partial: { ...this.snap.partial, peer: msg.text } });
            }
          });
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

    const deliver = (text: string, untranslated: boolean) => {
      if (this.isTerminal) return;
      // Phase 4: the listener cut into this turn — keep it readable, don't speak it.
      const interrupted = Boolean(this.listen?.isInterrupted(msg.turn));
      const held = !interrupted && Boolean(this.listen) && this.isLocalTalking();
      const record: SegmentRecord = {
        ...base,
        translated: text,
        translation: untranslated ? 'failed' : 'translated',
        untranslated: untranslated || undefined,
        ...(interrupted ? { interrupted: true } : {}),
        ...(held ? { overlapHeld: true } : {}),
      };
      this.addSegment(record);
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
        text,
        lang: untranslated ? msg.sourceLang : myLang,
        // Resolved at playback start → a revocation applies to queued segments too.
        voice: this.voice ? () => this.voice?.peerHints() : undefined,
        ...(this.conv ? { notAfter: receivedAt + this.conv.maxPlaybackAgeMs } : {}),
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
      this.deps
        .translate({ segmentId: msg.id, text: msg.original, from: msg.sourceLang, to: myLang, signal: this.work.signal })
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
    this.call?.send({ t: 'hello', lang: this.config.lang, name: this.config.userName, ...(this.conv ? { caps: [CONVERSATION_CAP] } : {}) });
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
        disclosure: route ? disclosureFor(route, this.server?.translator ?? null) : null,
      },
    });
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
      consent: { route, granted: this.privacy.hasConsent(route), prompt: true, disclosure: disclosureFor(route, this.server?.translator ?? null) },
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
