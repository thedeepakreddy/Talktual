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
  now?: () => number;
  setTimeout?: typeof setTimeout;
  clearTimeout?: typeof clearTimeout;
}

export interface Notice {
  code: ErrorCode | 'info' | 'untranslated-out' | 'untranslated-in' | 'peer-away' | 'degraded';
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
    };
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
    if (this.deps.playback.active) this.interrupt('barge-in');
    this.commit({ holding: true, partial: { ...this.snap.partial, local: '' } });
    this.startAsr('press');
  }

  releaseTalk(): void {
    if (!this.snap.holding) return;
    this.commit({ holding: false, hearingVoice: false });
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
        if (wasReconnecting) {
          this.emit({ type: 'recovery', kind: 'reconnected' });
          this.diag.record('reconnected', {});
        }
        if (this.snap.notice?.code === 'peer-away' || this.snap.notice?.code === 'connection-lost') this.clearNotice();
        this.sendHello();
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
    this.deps.capture.release();
    this.unsubs.forEach((u) => u());
    this.unsubs = [];
    if (this.call) {
      const c = this.call;
      this.call = null;
      c.close();
    }
    this.emit({ type: 'interruption', kind: 'leave', cancelledSegments: cancelled });
    this.commit({ holding: false, hearingVoice: false, playing: false, capturing: false, translating: 0, partial: { local: '', peer: '' } });
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
      this.asr = this.deps.createAsr(this.config.lang, this.privacy.mode, {
        onFinal: (seg) => this.onLocalFinal(seg),
        onPartial: (text) => this.onLocalPartial(text),
        onSpeechActivity: (active) => this.commit({ hearingVoice: active && (this.snap.holding || this.privacy.mode === 'continuous') }),
        onError: (code, message, fatal) => this.onAsrError(code, message, fatal),
        onEnd: () => this.onAsrEnd(),
      });
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
    if (how === 'abort' || disposing) {
      this.asr = null;
      asr.abort();
      asr.dispose();
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
      if (code !== 'permission-denied') {
        // Allow a retry on the next press.
        this.micEnabled = true;
      }
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
    const now = this.now();
    if (!text || now - this.lastPartialSentAt < PARTIAL_SEND_INTERVAL_MS) return;
    this.lastPartialSentAt = now;
    this.call?.send({ t: 'partial', text, lang: this.config.lang });
  }

  private onLocalFinal(seg: RecognizedSegment): void {
    if (this.isTerminal || !seg.text.trim()) return;
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
          original: record.original,
          translated: result.text,
          sourceLang,
          targetLang,
          sentAt: this.now(),
          ...(untranslated ? { untranslated: true } : {}),
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
      })
      .catch((err) => {
        this.diag.count('pipeline_errors');
        console.error('segment pipeline failed:', err);
      })
      .finally(() => {
        if (!this.disposed) this.commit({ translating: Math.max(0, this.snap.translating - 1) });
      });
  }

  // ==========================================================================
  // Incoming pipeline
  // ==========================================================================

  private onPeerMessage(msg: PeerMessage): void {
    if (this.isTerminal) return;
    const myLang = this.config.lang;

    if (msg.t === 'hello') {
      this.commit({ peer: { name: msg.name || null, lang: msg.lang } });
      this.emit({ type: 'peer.updated', name: msg.name || null, lang: msg.lang });
      return;
    }

    if (msg.t === 'partial') {
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

    // --- segment ---
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
    };

    const deliver = (text: string, untranslated: boolean) => {
      if (this.isTerminal) return;
      const record: SegmentRecord = {
        ...base,
        translated: text,
        translation: untranslated ? 'failed' : 'translated',
        untranslated: untranslated || undefined,
      };
      this.addSegment(record);
      if (untranslated) {
        this.setNotice({ code: 'untranslated-in', message: 'Translation unavailable — playing their original words.', sticky: false });
      }
      // Untranslated text is spoken with a voice for its own language;
      // reading it with ours would mangle every word.
      this.receivedAt.set(msg.id, receivedAt);
      this.deps.playback.enqueue({ segmentId: msg.id, text, lang: untranslated ? msg.sourceLang : myLang });
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
    if (status !== 'queued' && status !== 'started') this.receivedAt.delete(segmentId);
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

  private interrupt(kind: 'barge-in' | 'stop'): void {
    const cancelled = this.deps.playback.cancelAll();
    this.commit({ playing: false });
    this.emit({ type: 'interruption', kind, cancelledSegments: cancelled });
    this.diag.record('interruption', { kind, n: cancelled });
  }

  // ==========================================================================
  // Helpers
  // ==========================================================================

  private sendHello(): void {
    this.call?.send({ t: 'hello', lang: this.config.lang, name: this.config.userName });
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
