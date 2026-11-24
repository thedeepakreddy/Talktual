// Versioned contracts for the communication engine.
//
// Two families:
//   * EngineEvent — internal events the engine emits to its consumers (UI,
//     diagnostics). Typed, versioned, never sent over the network.
//   * PeerMessage — the wire protocol between the two browsers (data channel
//     or relay). External input: always validated before use.

import { isVoiceWireMessage, type VoiceWireMessage } from '../voicemirror/controller.ts';

export const ENGINE_EVENT_VERSION = 1 as const;
export const WIRE_VERSION = 1 as const;

// --- State vocabularies -------------------------------------------------------

export type CallPhase = 'idle' | 'connecting' | 'waiting-for-peer' | 'connected' | 'reconnecting' | 'ended' | 'failed';

export type Activity = 'idle' | 'permission-pending' | 'ready' | 'listening' | 'processing' | 'paused' | 'stopped';

export type ListeningMode = 'push-to-talk' | 'continuous';
/** Where captured audio is processed — what the user consents to. */
export type ProcessingRoute = 'browser-speech' | 'server-stt' | 'ce-local';
export type Transport = 'p2p' | 'relay' | 'none';
export type PermissionState = 'unknown' | 'prompt' | 'granted' | 'denied';

export type TranslationStatus = 'pending' | 'translated' | 'passthrough' | 'failed' | 'cancelled';
export type PlaybackStatus = 'queued' | 'started' | 'ended' | 'cancelled' | 'failed' | 'dropped';

export type ErrorCode =
  | 'permission-denied'
  | 'device-lost'
  | 'capture-unavailable'
  | 'stt-unavailable'
  | 'stt-language-unsupported'
  | 'stt-failed'
  | 'translation-failed'
  | 'playback-failed'
  | 'session-full'
  | 'session-invalid'
  | 'resume-failed'
  | 'connection-lost'
  | 'server-error';

// --- Segments -------------------------------------------------------------------

export interface SegmentRecord {
  /** Identity of the utterance end to end (sender-generated UUID). */
  id: string;
  /** Sender's monotonic sequence number, when known. */
  seq?: number;
  speaker: 'local' | 'peer';
  speakerName?: string;
  original: string;
  translated: string;
  sourceLang: string;
  targetLang: string;
  translation: TranslationStatus;
  playback?: PlaybackStatus;
  /** Translation failed: `translated` holds the original text. */
  untranslated?: boolean;
  /** Milliseconds from phrase final to translation done (local segments). */
  latencyMs?: number;
  // --- Phase 4 conversation layer (all optional) ---
  /** provisional = shown, never spoken, may still change; final = committed. */
  state?: 'provisional' | 'final';
  /** Revision of a provisional unit. */
  rev?: number;
  /** Speaking turn this unit belongs to. */
  turn?: string;
  /** The listener cut in: kept in the transcript, not spoken. */
  interrupted?: boolean;
  /** Recognizer confidence was low. */
  uncertain?: boolean;
  /** Self-repair (restart/repetition) changed the recognized words. */
  repaired?: boolean;
  /** Arrived while the local user was talking; played afterwards (or dropped if too old). */
  overlapHeld?: boolean;
  // --- Phase 5 communication intelligence (all optional, advisory) ---
  /** Transcript notes (never spoken): ambiguity, references, idioms, corrections, assistance. */
  notes?: Array<{ kind: string; text: string }>;
  /** Conversational signals (category + confidence), ≥ 0.5 only. */
  signals?: Array<{ category: string; confidence: number }>;
  /** The translation was context-assisted (explained in notes). */
  assisted?: boolean;
  /** A later turn corrected this one (id of the correcting segment). */
  correctedBy?: string;
  // --- Phase 6 processing location (advisory display) ---
  /** Where the translation was produced. */
  processedOn?: 'device' | 'cloud';
  /** Typed, not spoken. */
  typed?: boolean;
  /** The sender asked for on-device processing of these words. */
  privateRequest?: boolean;
  createdAt: number;
}

// --- Internal events --------------------------------------------------------------

type Ev<T extends string, P> = { v: typeof ENGINE_EVENT_VERSION; type: T; at: number } & P;

export type EngineEvent =
  | Ev<'session.phase', { from: CallPhase; to: CallPhase; reason?: string }>
  | Ev<'session.activity', { from: Activity; to: Activity; reason?: string }>
  | Ev<'transport.changed', { transport: Transport }>
  | Ev<'privacy.changed', { mode: ListeningMode; route: ProcessingRoute | null; consent: boolean; capturing: boolean }>
  | Ev<'permission.changed', { state: PermissionState }>
  | Ev<'peer.updated', { name: string | null; lang: string | null }>
  | Ev<'transcript.partial', { speaker: 'local' | 'peer'; text: string; lang: string }>
  | Ev<'transcript.final', { segment: SegmentRecord }>
  | Ev<'translation.updated', { segmentId: string; status: TranslationStatus; provider?: string; error?: string }>
  | Ev<'playback.status', { segmentId: string; status: PlaybackStatus; engine?: 'server' | 'browser' }>
  | Ev<'interruption', { kind: 'barge-in' | 'stop' | 'leave'; cancelledSegments: number; interruptedTurns?: number }>
  | Ev<'conversation.unit', { segmentId: string; state: 'held' | 'revised' | 'final' | 'discarded'; reason?: string }>
  | Ev<'recovery', { kind: 'reconnecting' | 'reconnected' | 'gave-up' | 'peer-away' | 'peer-back'; attempt?: number }>
  | Ev<'error', { code: ErrorCode; fatal: boolean; message: string }>;

export type EngineEventType = EngineEvent['type'];
export type EngineEventOf<T extends EngineEventType> = Extract<EngineEvent, { type: T }>;

type Payload<E> = E extends unknown ? Omit<E, 'v' | 'at'> : never;
/** Builds a stamped event; `at` can be injected for deterministic tests. */
export function makeEvent(e: Payload<EngineEvent>, at = Date.now()): EngineEvent {
  return { ...(e as object), v: ENGINE_EVENT_VERSION, at } as EngineEvent;
}

// --- Peer wire protocol v1 -------------------------------------------------------

export type PeerMessage =
  | { v?: 1; t: 'hello'; lang: string; name: string; /** Phase 4: capability strings, e.g. ['conv1']. */ caps?: string[] }
  | { v?: 1; t: 'partial'; text: string; lang: string; turn?: string; priv?: 'device' }
  | {
      v?: 1;
      t: 'segment';
      id: string;
      seq?: number;
      original: string;
      translated: string;
      sourceLang: string;
      targetLang: string;
      sentAt: number;
      /** Set when the sender's translation failed and `translated` is just the original. */
      untranslated?: boolean;
      // Phase 4 (sent only to peers that advertised 'conv1'; absent = final).
      state?: 'provisional' | 'final';
      rev?: number;
      turn?: string;
      uncertain?: boolean;
      repaired?: boolean;
      /** Phase 5 (sent only to 'aci1' peers): how the sender's translation was context-assisted. */
      assist?: string[];
      /** Phase 6 (sent only to 'priv1' peers): the sender is in on-device mode — keep processing of these words on-device. */
      priv?: 'device';
    }
  /** Phase 4: the sender opened/closed its speaking turn ("floor"). */
  | { v?: 1; t: 'floor'; turn: string; open: boolean }
  /** Phase 4: the sender interrupted the receiver's turn. */
  | { v?: 1; t: 'interrupt'; turn: string }
  /** Phase 4: a provisional unit was withdrawn (capture aborted before it became final). */
  | { v?: 1; t: 'retract'; id: string }
  | VoiceWireMessage;

const MAX_TEXT = 4000;
const isStr = (v: unknown, max: number): v is string => typeof v === 'string' && v.length <= max;
const isId = (v: unknown): v is string => isStr(v, 64) && v.length > 0;

/**
 * Runtime check for messages from the other peer. Everything arriving over the
 * data channel or the relay is untrusted input. Messages without `v` are
 * Phase 1 peers and are treated as v1; any other version is rejected.
 */
export function isPeerMessage(value: unknown): value is PeerMessage {
  if (!value || typeof value !== 'object') return false;
  const m = value as Record<string, unknown>;
  if (m.v !== undefined && m.v !== WIRE_VERSION) return false;
  switch (m.t) {
    case 'hello':
      return (
        isStr(m.lang, 35) &&
        isStr(m.name, 80) &&
        (m.caps === undefined || (Array.isArray(m.caps) && m.caps.length <= 16 && m.caps.every((c) => isStr(c, 16))))
      );
    case 'partial':
      return isStr(m.text, MAX_TEXT) && isStr(m.lang, 35) && (m.turn === undefined || isId(m.turn)) && (m.priv === undefined || m.priv === 'device');
    case 'floor':
      return isId(m.turn) && typeof m.open === 'boolean';
    case 'interrupt':
      return isId(m.turn);
    case 'retract':
      return isId(m.id);
    case 'segment':
      return (
        isStr(m.id, 64) &&
        m.id.length > 0 &&
        isStr(m.original, MAX_TEXT) &&
        isStr(m.translated, MAX_TEXT) &&
        isStr(m.sourceLang, 35) &&
        isStr(m.targetLang, 35) &&
        typeof m.sentAt === 'number' &&
        (m.seq === undefined || (Number.isInteger(m.seq) && (m.seq as number) >= 0)) &&
        (m.untranslated === undefined || typeof m.untranslated === 'boolean') &&
        (m.state === undefined || m.state === 'provisional' || m.state === 'final') &&
        (m.rev === undefined || (Number.isInteger(m.rev) && (m.rev as number) >= 0 && (m.rev as number) < 1000)) &&
        (m.turn === undefined || isId(m.turn)) &&
        (m.uncertain === undefined || typeof m.uncertain === 'boolean') &&
        (m.repaired === undefined || typeof m.repaired === 'boolean') &&
        (m.priv === undefined || m.priv === 'device') &&
        (m.assist === undefined ||
          (Array.isArray(m.assist) && m.assist.length <= 8 && m.assist.every((a) => typeof a === 'string' && /^(sense:[\p{L}'-]{1,30}:[a-z-]{1,20}|protect:\d{1,2})$/u.test(a))))
      );
    case 'voice':
      // Phase 3 VoiceMirror (additive; Phase 2 peers drop it as unknown).
      return isVoiceWireMessage(m);
    default:
      return false;
  }
}
