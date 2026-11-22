// Phase 4 conversation model: typed states for a unit of speech on its way
// from the recognizer to the listener, and the legal transitions between them.
//
//   speech-active ─final─▶ pause-candidate ─(hold)─▶ turn-candidate ─▶ final
//         │                    │  ▲   └─restart─▶ superseded (old revision)
//         │                    ▼  │
//         │                 provisional (held + provisional translation sent)
//         └─────────────── any non-final ─cancel─▶ superseded
//   final ─barge-in─▶ interrupted   (listener side: kept in transcript, not played)
//
// `uncertain` is a FLAG on a unit (low recognizer confidence), not a state:
// an uncertain unit still moves through the same states.

export type UnitState =
  | 'speech-active'
  | 'pause-candidate'
  | 'provisional'
  | 'turn-candidate'
  | 'final'
  | 'interrupted'
  | 'superseded';

export const UNIT_TRANSITIONS: Record<UnitState, readonly UnitState[]> = {
  'speech-active': ['pause-candidate', 'turn-candidate', 'superseded'],
  'pause-candidate': ['provisional', 'turn-candidate', 'superseded', 'pause-candidate'],
  provisional: ['provisional', 'turn-candidate', 'superseded'],
  'turn-candidate': ['final', 'superseded'],
  final: ['interrupted'],
  interrupted: [],
  superseded: [],
};

export function canTransition(from: UnitState, to: UnitState): boolean {
  return UNIT_TRANSITIONS[from].includes(to);
}

/** Why an open unit became final. Enum only — safe for diagnostics. */
export type CommitReason =
  | 'complete' // looked complete when the recognizer finalized it
  | 'hold-expired' // incomplete-looking, but the speaker did not continue in time
  | 'release' // push-to-talk released (explicit end-of-turn cue)
  | 'end-of-input' // recognizer stopped
  | 'max-length' // unit reached the size cap
  | 'max-open'; // unit was open for the hard maximum

/** Wire capability string: peers that understand Phase 4 messages. */
export const CONVERSATION_CAP = 'conv1';

export interface ConversationConfig {
  /** Minimum/maximum hold for an incomplete-looking fragment (ms). */
  minHoldMs: number;
  maxHoldMs: number;
  defaultHoldMs: number;
  /** Hard cap: a unit never stays open longer than this after it opened (ms). */
  maxOpenMs: number;
  /** After a push-to-talk release, the recognizer's flush is awaited at most this long (ms). */
  releaseFlushMs: number;
  /** Max characters in one unit (merge cap). */
  maxUnitChars: number;
  /** Below this recognizer confidence a unit is flagged uncertain (0..1). */
  uncertainBelow: number;
  /** Below this confidence a 1–2 word unit is treated as noise and dropped. */
  noiseBelow: number;
  /** Provisional translations: minimum spacing (ms). */
  provisionalMinIntervalMs: number;
  /** Translation latency above which the call is "degraded" (ms, smoothed). */
  degradedLatencyMs: number;
  /** Unplayed incoming speech older than this is dropped instead of played late (ms). */
  maxPlaybackAgeMs: number;
  /** Continuous mode: silence after which the speaker's floor is closed (ms). */
  floorIdleMs: number;
  /** Remembered interrupted turns (bounded). */
  maxInterruptedTurns: number;
}

export const DEFAULT_CONVERSATION_CONFIG: ConversationConfig = {
  minHoldMs: 500,
  maxHoldMs: 2000,
  defaultHoldMs: 900,
  maxOpenMs: 6000,
  releaseFlushMs: 1200,
  maxUnitChars: 400,
  uncertainBelow: 0.5,
  noiseBelow: 0.35,
  provisionalMinIntervalMs: 700,
  degradedLatencyMs: 2000,
  maxPlaybackAgeMs: 12000,
  floorIdleMs: 1500,
  maxInterruptedTurns: 64,
};
