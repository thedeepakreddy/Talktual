// Phase 5 — Autonomous Communication Intelligence (ACI): versioned contracts.
//
// ACI is advisory. It turns permitted session events into typed DECISIONS;
// the session validates and executes them (or falls back). ACI never touches
// capture, transport, playback queues, credentials or rendering.

export const ACI_VERSION = 1 as const;
/** Wire capability: the peer understands `assist` codes on segments. */
export const ACI_CAP = 'aci1';

export type AciLang = 'en' | 'es' | 'fr' | 'de' | 'pt' | 'it' | 'other';

export type EntityKind = 'person' | 'place' | 'name' | 'date' | 'time' | 'number' | 'term';

export interface ContextEntity {
  id: string;
  /** Surface form as spoken (proper names, dates, times — never phone/email/card-like strings). */
  text: string;
  kind: EntityKind;
  /** 'explicit' = pinned by the user; 'inferred' = extracted by rules (a guess, not a fact). */
  source: 'explicit' | 'inferred';
  /** Rule confidence 0..1 for inferred entities. */
  confidence: number;
  speaker: 'local' | 'peer';
  firstSeq: number;
  lastSeq: number;
  /** Superseded by a later correction (kept for linking, not used for resolution). */
  supersededBy?: string;
  segmentId: string;
}

export type SignalCategory =
  | 'question'
  | 'request'
  | 'offer'
  | 'confirmation'
  | 'refusal'
  | 'correction'
  | 'clarification-request'
  | 'misunderstanding'
  | 'urgency'
  | 'topic-change'
  | 'topic-continuation'
  | 'unresolved-reference';

export interface Signal {
  category: SignalCategory;
  /** Rule confidence 0..1 — a probabilistic suggestion, not a fact. */
  confidence: number;
}

export type NoteKind = 'ambiguity' | 'sense' | 'reference' | 'idiom' | 'correction' | 'assisted' | 'concise';

/** Shown in the transcript, never spoken, never sent. */
export interface Note {
  kind: NoteKind;
  /** Short UI text (the app's UI language). */
  text: string;
}

export type SuggestionKind = 'rephrase' | 'ambiguity' | 'reference';

/** Offered to the LOCAL user only; dismissible; never acts on its own. */
export interface Suggestion {
  id: string;
  kind: SuggestionKind;
  /** De-duplication key: a dismissed key is never offered again this session. */
  key: string;
  segmentId: string;
  text: string;
  createdAt: number;
  expiresAt: number;
}

export interface AciPrefs {
  /** Master switch. Off = context cleared, faithful context-free behaviour. */
  enabled: boolean;
  /** How incoming speech is delivered to me: faithful (default) or concise (discourse fillers removed). */
  mode: 'faithful' | 'concise';
  /** Context-assisted translation of my outgoing speech (name protection, sense pre-editing). */
  assist: boolean;
  /** Transcript notes (ambiguity, references, idioms, corrections). */
  notes: boolean;
  /** Suggestions to me. */
  suggestions: boolean;
}

export const DEFAULT_PREFS: AciPrefs = { enabled: true, mode: 'faithful', assist: true, notes: true, suggestions: true };

export interface AciLimits {
  maxTurns: number;
  maxTurnChars: number;
  maxEntities: number;
  maxTopic: number;
  maxSuggestions: number;
  maxDismissed: number;
  maxNotesPerSegment: number;
  /** Inputs longer than this are not analysed (bounded processing time) → faithful fallback. */
  maxAnalyzeChars: number;
  suggestionTtlMs: number;
}

export const DEFAULT_LIMITS: AciLimits = {
  maxTurns: 8,
  maxTurnChars: 1600,
  maxEntities: 48,
  maxTopic: 16,
  maxSuggestions: 3,
  maxDismissed: 64,
  maxNotesPerSegment: 3,
  maxAnalyzeChars: 600,
  suggestionTtlMs: 45_000,
};

/** One protected span in MT input: `token` must come back unchanged and is restored to `value`. */
export interface Protection {
  token: string;
  value: string;
}

/** A context-justified sense pre-edit of the MT input (never of what is shown or sent as original). */
export interface SenseEdit {
  word: string;
  senseId: string;
  replacement: string;
}

export interface OutboundDecision {
  v: typeof ACI_VERSION;
  type: 'outbound';
  segmentId: string;
  /** Text to give the translator. Equals the original unless assist is on and evidence supports a change. */
  mtInput: string;
  protections: Protection[];
  edits: SenseEdit[];
  /** Compact explanation codes sent to aci1 peers, e.g. "sense:bank:river", "protect:2". */
  assist: string[];
  signals: Signal[];
  /** Notes for my own transcript line (what was assisted and why). */
  notes: Note[];
  suggestions: Suggestion[];
  /** My turn corrects an earlier one of mine (annotated, never rewritten). */
  correction?: { segmentId: string; from: string; to: string };
  /** Set when ACI declined to act (e.g. input too long) — faithful context-free path. */
  fallback?: 'too-long' | 'disabled' | 'invalid';
}

export interface InboundDecision {
  v: typeof ACI_VERSION;
  type: 'inbound';
  segmentId: string;
  notes: Note[];
  signals: Signal[];
  /** Concise delivery (my preference): what to SPEAK; the transcript keeps the full text. */
  speak?: { text: string; removed: number };
  /** This turn corrects an earlier one (cross-turn): the earlier segment is annotated, never rewritten. */
  correction?: { segmentId: string; from: string; to: string };
  suggestions: Suggestion[];
  fallback?: 'too-long' | 'disabled' | 'invalid';
}

/** Inspectable, correctable context summary (no raw turns). */
export interface ContextSummary {
  entities: Array<Pick<ContextEntity, 'id' | 'text' | 'kind' | 'source' | 'confidence'> & { superseded: boolean }>;
  topic: string[];
  turns: number;
  chars: number;
  budget: { turns: number; chars: number; entities: number };
  langs: { local: string; peer: string | null };
}
