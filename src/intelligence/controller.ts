// IntelligenceController — the single entry point the session uses.
//
//   session event → extract → bounded context → decide → validate → (session executes)
//
// Synchronous, bounded, clock-injected. Holds only session-scoped state; every
// clear path (reset, disable, end) drops all of it.

import { detectAmbiguity, detectIdioms, resolveReferences, type AmbiguityFinding } from './analyze';
import { SessionContext } from './context';
import { extractEntities, type Extracted } from './extract';
import { AMBIGUOUS } from './lexicon';
import { concise, faithfulInbound, faithfulOutbound, validateInbound, validateOutbound } from './policy';
import { buildMtInput, restore, token } from './protect';
import { detectSignals } from './signals';
import { aciLang } from './text';
import {
  ACI_VERSION,
  DEFAULT_LIMITS,
  DEFAULT_PREFS,
  type AciLimits,
  type AciPrefs,
  type ContextEntity,
  type ContextSummary,
  type InboundDecision,
  type Note,
  type OutboundDecision,
  type Protection,
  type SenseEdit,
  type Signal,
  type Suggestion,
} from './types';

export interface AciStats {
  decisions: number;
  fallbacks: number;
  rejected: number;
  assisted: number;
  restoreFailures: number;
}

const firstGloss = (g: string) => g.split(' ').slice(0, 2).join(' ');
const quote = (s: string) => `“${s}”`;

export class IntelligenceController {
  readonly limits: AciLimits;
  private prefs: AciPrefs;
  private ctx: SessionContext;
  private suggestions: Suggestion[] = [];
  private dismissed: string[] = [];
  private nextSug = 0;
  /** Per segment: what ACI found (ids only — used to supersede suggestions). */
  private lastLocalSegment: string | null = null;
  private lastPeerWasClarification = false;
  stats: AciStats = { decisions: 0, fallbacks: 0, rejected: 0, assisted: 0, restoreFailures: 0 };

  constructor(
    private readonly now: () => number,
    opts: { prefs?: Partial<AciPrefs>; limits?: Partial<AciLimits> } = {}
  ) {
    this.limits = { ...DEFAULT_LIMITS, ...opts.limits };
    this.prefs = { ...DEFAULT_PREFS, ...opts.prefs };
    this.ctx = new SessionContext(this.limits);
  }

  // ---------------------------------------------------------------- controls

  getPrefs(): AciPrefs {
    return { ...this.prefs };
  }

  /** Turning intelligence off clears the context immediately (consent withdrawal). */
  setPrefs(patch: Partial<AciPrefs>): void {
    const next = { ...this.prefs, ...patch };
    if (next.mode !== 'faithful' && next.mode !== 'concise') next.mode = 'faithful';
    if (this.prefs.enabled && !next.enabled) this.reset();
    if (!next.suggestions) this.suggestions = [];
    this.prefs = next;
  }

  setLangs(local: string, peer: string | null): void {
    this.ctx.langs = { local, peer };
  }

  reset(): void {
    this.ctx.clear();
    this.suggestions = [];
    this.dismissed = [];
    this.lastLocalSegment = null;
    this.lastPeerWasClarification = false;
  }

  summary(): ContextSummary {
    return this.ctx.summary();
  }

  get maxObserved() {
    return this.ctx.maxObserved;
  }

  removeEntity(id: string): boolean {
    return this.ctx.remove(id);
  }

  pinTerm(text: string): ContextEntity | null {
    return this.prefs.enabled ? this.ctx.pin(text) : null;
  }

  activeSuggestions(): Suggestion[] {
    const t = this.now();
    this.suggestions = this.suggestions.filter((s) => s.expiresAt > t);
    return [...this.suggestions];
  }

  dismiss(id: string): void {
    const s = this.suggestions.find((x) => x.id === id);
    if (!s) return;
    this.suggestions = this.suggestions.filter((x) => x.id !== id);
    this.rememberDismissed(s.key);
  }

  /** A segment was interrupted, retracted or superseded: its suggestions go away. */
  dropSegment(segmentId: string): void {
    this.suggestions = this.suggestions.filter((s) => s.segmentId !== segmentId);
  }

  // ---------------------------------------------------------------- outbound (my speech)

  outbound(input: { segmentId: string; text: string; lang: string; targetLang: string }): OutboundDecision {
    this.stats.decisions++;
    if (!this.prefs.enabled) return this.fallbackOut(input.segmentId, input.text, 'disabled');
    if (input.text.length > this.limits.maxAnalyzeChars) {
      this.ingest(input.segmentId, 'local', input.text, input.lang, []);
      return this.fallbackOut(input.segmentId, input.text, 'too-long');
    }
    const lang = aciLang(input.lang);
    const recent = this.ctx.recent().map((t) => t.text);
    const found = extractEntities(input.text, lang, (t) => this.ctx.knows(t));
    const signals = this.signalsFor(input.text, lang, 'local');
    const amb = detectAmbiguity(input.text, lang, recent);
    const refs = resolveReferences(input.text, lang, this.ctx, { topicBreak: signals.some((s) => s.category === 'topic-change' && s.confidence >= 0.8), currentEntities: [] });
    if (refs.some((r) => !r.antecedent && r.candidates >= 2)) signals.push({ category: 'unresolved-reference', confidence: 0.6 });

    const protections: Protection[] = [];
    const edits: SenseEdit[] = [];
    const notes: Note[] = [];
    const assist: string[] = [];
    if (this.prefs.assist) {
      const pinned = this.ctx.entityList(['term']).filter((e) => e.source === 'explicit' && input.text.includes(e.text));
      const names = [...pinned.map((e) => e.text), ...found.filter((e) => e.protect).map((e) => e.text)];
      // longest first so "Calle Mayor" wins over "Mayor"
      for (const value of [...new Set(names)].sort((a, b) => b.length - a.length).slice(0, 8)) {
        if (protections.some((p) => p.value.includes(value))) continue;
        protections.push({ token: token(protections.length), value });
      }
      for (const f of amb) {
        if (f.sense?.edit) edits.push({ word: f.surface, senseId: f.sense.id, replacement: f.sense.edit });
      }
      if (protections.length) assist.push(`protect:${protections.length}`);
      for (const e of edits) assist.push(`sense:${e.word}:${e.senseId}`);
      if (edits.length) notes.push({ kind: 'assisted', text: `Context-assisted: translated ${edits.map((e) => `${quote(e.word)} as ${quote(e.replacement)}`).join(', ')}` });
      if (protections.length) notes.push({ kind: 'assisted', text: `Kept as spoken: ${protections.map((p) => p.value).join(', ')}` });
    }
    const suggestions = this.prefs.suggestions ? this.outboundSuggestions(input.segmentId, amb, refs) : [];
    const correction = this.linkCorrection(signals, found, 'local');
    const decision: OutboundDecision = {
      v: ACI_VERSION,
      type: 'outbound',
      segmentId: input.segmentId,
      mtInput: buildMtInput(input.text, protections, edits),
      protections,
      edits,
      assist,
      signals,
      notes: notes.slice(0, this.limits.maxNotesPerSegment),
      suggestions,
      ...(correction ? { correction } : {}),
    };
    this.ingest(input.segmentId, 'local', input.text, input.lang, found);
    this.lastLocalSegment = input.segmentId;
    if (!validateOutbound(decision, input.text, AMBIGUOUS[lang], this.limits)) {
      this.stats.rejected++;
      return this.fallbackOut(input.segmentId, input.text, 'invalid');
    }
    if (protections.length || edits.length) this.stats.assisted++;
    this.addSuggestions(suggestions);
    return decision;
  }

  /** Restore protected names in the translation. ok=false → caller uses the context-free path. */
  restoreTranslation(d: OutboundDecision, translated: string): { text: string; ok: boolean } {
    const r = restore(translated, d.protections);
    if (!r.ok) this.stats.restoreFailures++;
    return r;
  }

  // ---------------------------------------------------------------- inbound (their speech)

  inbound(input: { segmentId: string; original: string; sourceLang: string; translated: string; listenerLang: string; assist?: string[] }): InboundDecision {
    this.stats.decisions++;
    if (!this.prefs.enabled) return this.fallbackIn(input.segmentId, 'disabled');
    if (input.original.length > this.limits.maxAnalyzeChars) {
      this.ingest(input.segmentId, 'peer', input.original, input.sourceLang, []);
      return this.fallbackIn(input.segmentId, 'too-long');
    }
    const lang = aciLang(input.sourceLang);
    const recent = this.ctx.recent().map((t) => t.text);
    const found = extractEntities(input.original, lang, (t) => this.ctx.knows(t));
    const signals = this.signalsFor(input.original, lang, 'peer');
    const topicBreak = signals.some((s) => s.category === 'topic-change' && s.confidence >= 0.8);
    const amb = detectAmbiguity(input.original, lang, recent);
    const idioms = detectIdioms(input.original, lang);
    const refs = resolveReferences(input.original, lang, this.ctx, { topicBreak, currentEntities: [] });
    if (refs.some((r) => !r.antecedent && r.candidates >= 2)) signals.push({ category: 'unresolved-reference', confidence: 0.6 });
    const correction = this.linkCorrection(signals, found, 'peer');

    const notes: Note[] = [];
    if (this.prefs.notes) {
      if (correction) notes.push({ kind: 'correction', text: `Corrects earlier: ${correction.from} → ${correction.to}` });
      for (const code of input.assist ?? []) {
        const m = code.match(/^sense:(.+):(.+)$/);
        const sense = m && AMBIGUOUS[lang]?.find((w) => w.word === m[1] || w.forms?.includes(m[1]))?.senses.find((s) => s.id === m[2]);
        if (m && sense) notes.push({ kind: 'assisted', text: `Their app used context: ${quote(m[1])} meant ${firstGloss(sense.gloss)}` });
      }
      for (const f of amb) {
        if (input.assist?.some((a) => a.startsWith(`sense:${f.surface}:`))) continue;
        if (f.sense) notes.push({ kind: 'sense', text: `${quote(f.surface)} here likely means ${firstGloss(f.sense.gloss)} (from context)` });
        else notes.push({ kind: 'ambiguity', text: `${quote(f.surface)} can mean ${f.entry.senses.map((s) => firstGloss(s.gloss)).join(' or ')}` });
      }
      for (const r of refs) if (r.antecedent) notes.push({ kind: 'reference', text: `${quote(r.word)} → ${r.antecedent.text} (mentioned earlier)` });
      for (const i of idioms) notes.push({ kind: 'idiom', text: `${quote(i.phrase)} is an idiom: ${i.meaning}` });
    }
    let speak: InboundDecision['speak'];
    if (this.prefs.mode === 'concise') {
      const c = concise(input.translated, aciLang(input.listenerLang));
      if (c.removed > 0 && c.text) {
        speak = c;
        notes.push({ kind: 'concise', text: `Concise: ${c.removed} filler${c.removed > 1 ? 's' : ''} not spoken` });
      }
    }
    const misunderstood = signals.some((s) => s.category === 'misunderstanding');
    const suggestions: Suggestion[] = [];
    if (this.prefs.suggestions && misunderstood && this.lastLocalSegment) {
      const s = this.makeSuggestion('rephrase', `rephrase:${this.lastLocalSegment}`, this.lastLocalSegment, 'They may not have understood your last message — try saying it more simply.');
      if (s) suggestions.push(s);
    }
    const decision: InboundDecision = {
      v: ACI_VERSION,
      type: 'inbound',
      segmentId: input.segmentId,
      notes: notes.slice(0, this.limits.maxNotesPerSegment + (speak ? 1 : 0)).slice(0, this.limits.maxNotesPerSegment),
      signals,
      ...(speak ? { speak } : {}),
      ...(correction ? { correction } : {}),
      suggestions,
    };
    this.lastPeerWasClarification = signals.some((s) => s.category === 'clarification-request');
    this.ingest(input.segmentId, 'peer', input.original, input.sourceLang, found);
    if (!validateInbound(decision, input.translated, this.limits)) {
      this.stats.rejected++;
      return this.fallbackIn(input.segmentId, 'invalid');
    }
    this.addSuggestions(suggestions);
    return decision;
  }

  // ---------------------------------------------------------------- internals

  private signalsFor(text: string, lang: ReturnType<typeof aciLang>, speaker: 'local' | 'peer'): Signal[] {
    const last = this.ctx.lastTurn();
    return detectSignals(text, lang, {
      previousByOther: Boolean(last && last.speaker !== speaker),
      previousWasClarification: speaker === 'local' ? this.lastPeerWasClarification : false,
      topicWords: this.ctx.topicWords(),
      turnsSoFar: this.ctx.size.turns,
    });
  }

  /** "No, Tuesday" — the newest same-kind entity from the same speaker is superseded. */
  private linkCorrection(signals: Signal[], found: Extracted[], speaker: 'local' | 'peer'): InboundDecision['correction'] {
    if (!signals.some((s) => s.category === 'correction')) return undefined;
    // The corrected value is the LAST one of its kind in the turn ("not 5 pm, 6 pm").
    const lastOfKind = new Map<string, Extracted>();
    for (const e of found) lastOfKind.set(e.kind, e);
    for (const cur of lastOfKind.values()) {
      if (!['date', 'time', 'number', 'place', 'person', 'name'].includes(cur.kind)) continue;
      const prev = this.ctx
        .entityList([cur.kind])
        .filter((e) => e.speaker === speaker && !e.supersededBy && e.text.toLowerCase() !== cur.text.toLowerCase())
        .sort((a, b) => b.lastSeq - a.lastSeq)[0];
      if (prev) {
        this.ctx.supersede(prev.id, cur.text);
        return { segmentId: prev.segmentId, from: prev.text, to: cur.text };
      }
    }
    return undefined;
  }

  private outboundSuggestions(segmentId: string, amb: AmbiguityFinding[], refs: ReturnType<typeof resolveReferences>): Suggestion[] {
    const out: Suggestion[] = [];
    for (const f of amb) {
      if (f.sense) continue;
      const s = this.makeSuggestion('ambiguity', `amb:${f.entry.word}`, segmentId, `${quote(f.surface)} can mean ${f.entry.senses.map((x) => firstGloss(x.gloss)).join(' or ')} — add a word if it matters.`);
      if (s) out.push(s);
    }
    for (const r of refs) {
      if (r.antecedent || r.candidates < 2) continue;
      const s = this.makeSuggestion('reference', `ref:${r.word}`, segmentId, `${quote(r.word)} could point to more than one ${r.kind} — say which if it matters.`);
      if (s) out.push(s);
    }
    return out.slice(0, 1);
  }

  private makeSuggestion(kind: Suggestion['kind'], key: string, segmentId: string, text: string): Suggestion | null {
    // Loop guard: a dismissed or currently shown key is never offered again.
    if (this.dismissed.includes(key) || this.suggestions.some((s) => s.key === key)) return null;
    const t = this.now();
    return { id: `s${++this.nextSug}`, kind, key, segmentId, text: text.slice(0, 200), createdAt: t, expiresAt: t + this.limits.suggestionTtlMs };
  }

  private addSuggestions(list: Suggestion[]): void {
    for (const s of list) {
      // Same kind supersedes the older one (no stacking).
      this.suggestions = this.suggestions.filter((x) => x.kind !== s.kind);
      this.suggestions.push(s);
      // Shown once: after it expires it is not offered again either.
      this.rememberDismissed(s.key);
    }
    while (this.suggestions.length > this.limits.maxSuggestions) this.suggestions.shift();
  }

  private rememberDismissed(key: string): void {
    if (!this.dismissed.includes(key)) this.dismissed.push(key);
    while (this.dismissed.length > this.limits.maxDismissed) this.dismissed.shift();
  }

  private ingest(segmentId: string, speaker: 'local' | 'peer', text: string, lang: string, found: Extracted[]): void {
    this.ctx.addTurn(segmentId, speaker, text, lang, found);
  }

  private fallbackOut(segmentId: string, text: string, why: OutboundDecision['fallback']): OutboundDecision {
    this.stats.fallbacks++;
    return faithfulOutbound(segmentId, text, why);
  }

  private fallbackIn(segmentId: string, why: InboundDecision['fallback']): InboundDecision {
    this.stats.fallbacks++;
    return faithfulInbound(segmentId, why);
  }
}
