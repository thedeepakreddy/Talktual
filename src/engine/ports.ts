// Interfaces the engine depends on. Concrete adapters live in src/lib; tests
// inject fakes. Keeping these here (not in the adapters) is what lets the
// session be tested without a browser, a network or a provider.

import type { ListeningMode, PermissionState, PlaybackStatus, ProcessingRoute } from './events';
import type { CallHandlers, CallHandle } from '../lib/webrtc';

// --- Speech recognition --------------------------------------------------------

export type AsrEngine = 'webspeech' | 'server-stt';

export interface RecognizedSegment {
  text: string;
  /** BCP-47 tag the recognizer was asked for. */
  lang: string;
  /** ms epoch when capture for this phrase began (best effort). */
  startedAt: number;
  /** ms epoch when the phrase was finalized. */
  endedAt: number;
  engine: AsrEngine;
}

export type AsrErrorCode = 'permission-denied' | 'device-lost' | 'capture-unavailable' | 'network' | 'no-result' | 'provider' | 'unknown';

export interface AsrCallbacks {
  onFinal: (segment: RecognizedSegment) => void;
  onPartial: (text: string) => void;
  onSpeechActivity?: (active: boolean) => void;
  onError: (code: AsrErrorCode, message: string, fatal: boolean) => void;
  /** The recognizer has fully stopped (after stop()/abort() or a fatal error). */
  onEnd?: () => void;
}

/**
 * A recognizer listens only between start() and stop()/abort(). It never starts
 * on its own — that is what makes push-to-talk privacy enforceable.
 */
export interface AsrAdapter {
  readonly engine: AsrEngine;
  readonly route: ProcessingRoute;
  readonly active: boolean;
  start(): Promise<void>;
  /** Graceful: keep accepting for the release tail, flush the last phrase, then end. */
  stop(): void;
  /** Immediate: discard anything in flight, release capture. */
  abort(): void;
  dispose(): void;
}

export type AsrFactory = (lang: string, mode: ListeningMode, cb: AsrCallbacks) => AsrAdapter;

// --- Translation ---------------------------------------------------------------------

export interface TranslationRequest {
  segmentId: string;
  text: string;
  from: string;
  to: string;
  signal?: AbortSignal;
}

export type TranslationErrorCode = 'timeout' | 'network' | 'http' | 'rate-limited' | 'malformed' | 'cancelled';

export interface TranslationResult {
  segmentId: string;
  status: 'translated' | 'passthrough' | 'failed' | 'cancelled';
  /** The translation; on failure/cancel this is the ORIGINAL text and `status` says so. */
  text: string;
  error?: { code: TranslationErrorCode; message: string };
  attempts: number;
  latencyMs: number;
}

export type Translator = (req: TranslationRequest) => Promise<TranslationResult>;

// --- Playback --------------------------------------------------------------------------

export interface PlaybackItem {
  segmentId: string;
  text: string;
  lang: string;
}

export type PlaybackListener = (segmentId: string, status: PlaybackStatus, engine?: 'server' | 'browser') => void;

export interface PlaybackPort {
  enqueue(item: PlaybackItem): void;
  /** Cancels the playing segment and drops the queue. Returns how many were cancelled. */
  cancelAll(): number;
  readonly active: boolean;
  onStatus(fn: PlaybackListener): () => void;
}

// --- Capture -----------------------------------------------------------------------------

export interface CapturePort {
  permission(): Promise<PermissionState>;
  release(): void;
  onDeviceLost(fn: () => void): () => void;
}

// --- Transport / server ------------------------------------------------------------------

export type ConnectFn = (sessionId: string, url: string, handlers: CallHandlers) => CallHandle;

export interface ServerCapabilities {
  tts: boolean;
  stt: boolean;
  translator: string;
  liveCaptions: boolean;
}

export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}
