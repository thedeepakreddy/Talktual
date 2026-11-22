// Bounded, session-scoped conversation context. Deterministic budgets and
// eviction; nothing persisted; sensitive identifiers redacted before storage.

import { STOP } from './lexicon';
import { redactSensitive, words } from './text';
import type { AciLimits, ContextEntity, ContextSummary, EntityKind } from './types';
import type { Extracted } from './extract';

export interface Turn {
  segmentId: string;
  speaker: 'local' | 'peer';
  /** Redacted text (no phone/email/card-like strings). Memory only, never logged. */
  text: string;
  lang: string;
  seq: number;
}

export class SessionContext {
  private turns: Turn[] = [];
  private entities = new Map<string, ContextEntity>();
  private topic = new Map<string, number>();
  private seq = 0;
  private nextId = 0;
  /** Words the user removed from context (never re-added this session). */
  private blocked = new Set<string>();
  /** Observed maxima (evaluation). */
  maxObserved = { turns: 0, chars: 0, entities: 0 };
  langs: { local: string; peer: string | null } = { local: '', peer: null };

  constructor(private readonly limits: AciLimits) {}

  get size(): { turns: number; chars: number; entities: number } {
    return { turns: this.turns.length, chars: this.turns.reduce((a, t) => a + t.text.length, 0), entities: this.entities.size };
  }

  addTurn(segmentId: string, speaker: 'local' | 'peer', raw: string, lang: string, found: Extracted[]): Turn {
    const text = redactSensitive(raw).text.slice(0, this.limits.maxTurnChars);
    const turn: Turn = { segmentId, speaker, text, lang, seq: ++this.seq };
    this.turns.push(turn);
    // Evict oldest turns first by count, then by character budget.
    while (this.turns.length > this.limits.maxTurns) this.turns.shift();
    while (this.size.chars > this.limits.maxTurnChars && this.turns.length > 1) this.turns.shift();
    for (const e of found) this.upsertEntity(e, speaker, turn.seq, segmentId);
    for (const w of words(text)) {
      if (w.length < 4 || STOP.has(w) || /\d/.test(w)) continue;
      this.topic.set(w, (this.topic.get(w) ?? 0) + 1);
    }
    // Topic keeps the top-N by count (ties: most recent insertion wins).
    if (this.topic.size > this.limits.maxTopic) {
      const keep = [...this.topic.entries()].sort((a, b) => b[1] - a[1]).slice(0, this.limits.maxTopic);
      this.topic = new Map(keep);
    }
    const s = this.size;
    this.maxObserved = {
      turns: Math.max(this.maxObserved.turns, s.turns),
      chars: Math.max(this.maxObserved.chars, s.chars),
      entities: Math.max(this.maxObserved.entities, s.entities),
    };
    return turn;
  }

  private upsertEntity(e: Extracted, speaker: 'local' | 'peer', seq: number, segmentId: string): void {
    const key = e.text.toLowerCase();
    if (this.blocked.has(key)) return;
    const existing = this.entities.get(key);
    if (existing) {
      // Same surface form: keep one entry; a specific kind (person/place) beats an unknown 'name'.
      if (existing.kind === 'name' && (e.kind === 'person' || e.kind === 'place')) existing.kind = e.kind;
      existing.lastSeq = seq;
      existing.confidence = Math.min(0.95, existing.confidence + 0.1);
      // refresh LRU order
      this.entities.delete(key);
      this.entities.set(key, existing);
      return;
    }
    this.entities.set(key, {
      id: `e${++this.nextId}`,
      text: e.text,
      kind: e.kind,
      source: 'inferred',
      confidence: e.confidence,
      speaker,
      firstSeq: seq,
      lastSeq: seq,
      segmentId,
    });
    // LRU eviction; explicit (pinned) entities are evicted last.
    while (this.entities.size > this.limits.maxEntities) {
      const victim = [...this.entities.entries()].find(([, v]) => v.source === 'inferred') ?? this.entities.entries().next().value!;
      this.entities.delete(victim[0]);
    }
  }

  /** Recent turns, oldest first, excluding the given segment. */
  recent(excludeSegmentId?: string): Turn[] {
    return this.turns.filter((t) => t.segmentId !== excludeSegmentId);
  }

  lastTurn(): Turn | undefined {
    return this.turns.at(-1);
  }

  entityList(kinds?: EntityKind[]): ContextEntity[] {
    const all = [...this.entities.values()];
    return kinds ? all.filter((e) => kinds.includes(e.kind)) : all;
  }

  /** Has this surface form been seen as a name before? */
  knows(text: string): boolean {
    const t = text.toLowerCase();
    for (const e of this.entities.values()) if (e.text.toLowerCase() === t) return true;
    return false;
  }

  topicWords(): string[] {
    return [...this.topic.keys()];
  }

  supersede(entityId: string, byText: string): void {
    for (const e of this.entities.values()) if (e.id === entityId) e.supersededBy = byText;
  }

  /** User correction: forget an item now and don't re-learn it this session. */
  remove(entityId: string): boolean {
    for (const [k, e] of this.entities) {
      if (e.id === entityId) {
        this.entities.delete(k);
        this.blocked.add(e.text.toLowerCase());
        if (this.blocked.size > 64) this.blocked.delete(this.blocked.values().next().value!);
        return true;
      }
    }
    return false;
  }

  /** User pin: keep this name verbatim in translation. */
  pin(text: string, kind: EntityKind = 'term'): ContextEntity | null {
    const clean = redactSensitive(text).text.trim().slice(0, 60);
    if (!clean || clean.includes('[number]')) return null;
    this.blocked.delete(clean.toLowerCase());
    const key = clean.toLowerCase();
    const e: ContextEntity = { id: `e${++this.nextId}`, text: clean, kind, source: 'explicit', confidence: 1, speaker: 'local', firstSeq: this.seq, lastSeq: this.seq, segmentId: '' };
    this.entities.set(key, e);
    while (this.entities.size > this.limits.maxEntities) {
      const victim = [...this.entities.entries()].find(([, v]) => v.source === 'inferred');
      if (!victim) break;
      this.entities.delete(victim[0]);
    }
    return e;
  }

  summary(): ContextSummary {
    const s = this.size;
    return {
      entities: [...this.entities.values()].map((e) => ({ id: e.id, text: e.text, kind: e.kind, source: e.source, confidence: e.confidence, superseded: Boolean(e.supersededBy) })),
      topic: this.topicWords().slice(0, 8),
      turns: s.turns,
      chars: s.chars,
      budget: { turns: this.limits.maxTurns, chars: this.limits.maxTurnChars, entities: this.limits.maxEntities },
      langs: { ...this.langs },
    };
  }

  /** Everything gone (reset / disable / session end). */
  clear(): void {
    this.turns = [];
    this.entities.clear();
    this.topic.clear();
    this.blocked.clear();
    this.seq = 0;
  }
}
