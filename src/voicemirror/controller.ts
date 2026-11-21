// VoiceMirrorController — per-call state for the "characteristics" tier.
//
// Lifecycle (all in memory, per call; nothing persisted):
//   off ──optIn()──▶ on: while capture is active the tap feeds the analyzer;
//                       after each capture stop, a changed descriptor is sent
//   on ──optOut()──▶ off: analyzer reset, `revoked` sent, nothing kept
//   peer descriptor ── received (validated) → used for playback hints
//                   ── peer revoked / call ends → deleted immediately
// Diagnostics receive enum/boolean status only — never pitch, tempo or any
// number derived from the voice.

import { VoiceAnalyzer } from './analyzer.ts';
import { isVoiceDescriptor, toDescriptor, toRenderHints } from './descriptor.ts';
import type { RenderHints, VoiceDescriptor } from './types.ts';

/** Opens a short-lived audio stream for analysis. Must only be called while capture is allowed. */
export interface VoiceTapPort {
  /** Starts delivering mono chunks. Resolves to a stop function. */
  start(onChunk: (samples: Float32Array, sampleRate: number) => void): Promise<() => void>;
}

export type VoiceWireMessage = { v?: 1; t: 'voice'; descriptor?: VoiceDescriptor; revoked?: boolean };

export function isVoiceWireMessage(x: unknown): x is VoiceWireMessage {
  if (!x || typeof x !== 'object') return false;
  const m = x as Record<string, unknown>;
  if (m.t !== 'voice' || (m.v !== undefined && m.v !== 1)) return false;
  const keys = Object.keys(m).filter((k) => k !== 'v' && k !== 't');
  if (m.revoked === true) return keys.length === 1;
  return keys.length === 1 && isVoiceDescriptor(m.descriptor);
}

export type VoiceDiagStatus =
  | 'opt-in'
  | 'opt-out'
  | 'tap-start'
  | 'tap-stop'
  | 'tap-failed'
  | 'descriptor-sent'
  | 'descriptor-withheld'
  | 'peer-descriptor'
  | 'peer-revoked'
  | 'peer-rejected'
  | 'cleared';

export interface VoiceMirrorState {
  /** Local user shares their voice characteristics this call. */
  sharing: boolean;
  /** A descriptor has actually been sent (enough speech was analysed). */
  descriptorSent: boolean;
  /** The peer shared characteristics; their speech is rendered with them. */
  peerActive: boolean;
}

export class VoiceMirrorController {
  private analyzer: VoiceAnalyzer | null = null;
  private analyzerRate = 0;
  private stopTap: (() => void) | null = null;
  private tapEpoch = 0;
  private sent: VoiceDescriptor | null = null;
  private peer: VoiceDescriptor | null = null;
  private _sharing = false;
  private disposed = false;

  constructor(
    private readonly deps: {
      tap: VoiceTapPort;
      send: (m: VoiceWireMessage) => void;
      diag: (status: VoiceDiagStatus) => void;
      onChange: () => void;
    }
  ) {}

  get state(): VoiceMirrorState {
    return { sharing: this._sharing, descriptorSent: this.sent !== null, peerActive: this.peer !== null };
  }

  optIn(): void {
    if (this.disposed || this._sharing) return;
    this._sharing = true;
    this.deps.diag('opt-in');
    this.deps.onChange();
  }

  /** Revocation: stop analysing, forget everything, tell the peer to forget too. */
  optOut(): void {
    if (this.disposed || !this._sharing) return;
    this._sharing = false;
    this.stopCapture(false);
    this.analyzer = null;
    const hadSent = this.sent !== null;
    this.sent = null;
    if (hadSent) this.deps.send({ t: 'voice', revoked: true });
    this.deps.diag('opt-out');
    this.deps.onChange();
  }

  /** Session: capture just became active (press / hands-free listening). */
  onCaptureStart(): void {
    if (this.disposed || !this._sharing || this.stopTap) return;
    const epoch = ++this.tapEpoch;
    this.deps
      .tap.start((samples, rate) => {
        if (epoch !== this.tapEpoch || !this._sharing) return;
        if (!this.analyzer || this.analyzerRate !== rate) {
          this.analyzer = new VoiceAnalyzer(rate);
          this.analyzerRate = rate;
        }
        this.analyzer.push(samples);
      })
      .then((stop) => {
        if (epoch !== this.tapEpoch || !this._sharing || this.disposed) {
          stop(); // capture ended (or opt-out) before the tap opened
          return;
        }
        this.stopTap = stop;
        this.deps.diag('tap-start');
      })
      .catch(() => this.deps.diag('tap-failed'));
  }

  /** Session: capture stopped. Publishes the descriptor if it changed. */
  onCaptureStop(): void {
    this.stopCapture(true);
  }

  private stopCapture(publish: boolean): void {
    this.tapEpoch++;
    if (this.stopTap) {
      const stop = this.stopTap;
      this.stopTap = null;
      try {
        stop();
      } catch {
        /* ignore */
      }
      this.deps.diag('tap-stop');
    }
    if (!publish || !this._sharing || !this.analyzer) return;
    const d = toDescriptor(this.analyzer.profile());
    if (!d) {
      this.deps.diag('descriptor-withheld'); // not enough confident speech yet
      return;
    }
    if (this.sent && sameDescriptor(this.sent, d)) return;
    this.sent = d;
    this.deps.send({ t: 'voice', descriptor: d });
    this.deps.diag('descriptor-sent');
    this.deps.onChange();
  }

  /** Session: (re)connected — the peer may have missed our last message. */
  onConnected(): void {
    if (this.disposed) return;
    if (this._sharing && this.sent) this.deps.send({ t: 'voice', descriptor: this.sent });
  }

  onPeerMessage(m: unknown): void {
    if (this.disposed) return;
    if (!isVoiceWireMessage(m)) {
      this.deps.diag('peer-rejected');
      return;
    }
    if (m.revoked) {
      this.peer = null;
      this.deps.diag('peer-revoked');
    } else {
      this.peer = m.descriptor!;
      this.deps.diag('peer-descriptor');
    }
    this.deps.onChange();
  }

  /** Resolved when a segment STARTS playing, so a revocation applies to everything not yet started. */
  peerHints(): RenderHints | undefined {
    return this.peer ? toRenderHints(this.peer) : undefined;
  }

  /** Session end: everything goes. Idempotent. */
  dispose(): void {
    if (this.disposed) return;
    this.stopCapture(false);
    this.analyzer = null;
    this.sent = null;
    this.peer = null;
    this._sharing = false;
    this.disposed = true;
    this.deps.diag('cleared');
  }

  /** For tests: is any voice-derived data still held? */
  holdsVoiceData(): boolean {
    return this.analyzer !== null || this.sent !== null || this.peer !== null || this.stopTap !== null;
  }
}

function sameDescriptor(a: VoiceDescriptor, b: VoiceDescriptor): boolean {
  return a.pitchStep === b.pitchStep && a.pitchRange === b.pitchRange && a.tempoPct === b.tempoPct;
}
