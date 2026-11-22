// Ambiguity, idiom and reference analysis on ONE utterance plus bounded context.
// Never changes text; returns findings with their evidence.

import { AMBIGUOUS, IDIOMS, REFERENCES, type AmbiguousWord, type Idiom, type Sense } from './lexicon';
import { hasPhrase, words } from './text';
import type { AciLang, ContextEntity } from './types';
import type { SessionContext } from './context';

export interface AmbiguityFinding {
  /** The word as it appears in the text. */
  surface: string;
  entry: AmbiguousWord;
  /** Selected sense when context evidence is sufficient; otherwise undefined (stay ambiguous). */
  sense?: Sense;
  scores: Record<string, number>;
}

/** Evidence rule: in-utterance cue = 2, cue in recent context = 1. Resolve only if best ≥ 2 and best ≥ 2× runner-up. */
export function detectAmbiguity(text: string, lang: AciLang, contextText: string[]): AmbiguityFinding[] {
  const lex = AMBIGUOUS[lang];
  if (!lex) return [];
  const toks = words(text);
  const ctx = new Set(contextText.flatMap((t) => words(t)));
  const out: AmbiguityFinding[] = [];
  for (const entry of lex) {
    const forms = [entry.word, ...(entry.forms ?? [])];
    const surface = toks.find((t) => forms.includes(t));
    if (!surface) continue;
    // A capitalized occurrence mid-sentence is a name ("Bill", "Rose"), not this word.
    if (onlyAsName(text, surface)) continue;
    const own = new Set(toks.filter((t) => !forms.includes(t)));
    const scores: Record<string, number> = {};
    for (const s of entry.senses) {
      let score = 0;
      for (const cue of s.cues) {
        if (cue.includes(' ') ? hasPhrase(text, cue) : own.has(cue)) score += 2;
        else if (ctx.has(cue)) score += 1;
      }
      scores[s.id] = score;
    }
    const ranked = entry.senses.map((s) => [s, scores[s.id]] as const).sort((a, b) => b[1] - a[1]);
    const [best, second] = ranked;
    const resolved = best[1] >= 2 && best[1] >= 2 * (second?.[1] ?? 0);
    out.push({ surface, entry, sense: resolved ? best[0] : undefined, scores });
  }
  return out;
}

/** True when every occurrence of `w` is capitalized and not sentence-initial (i.e. used as a name). */
function onlyAsName(text: string, w: string): boolean {
  const re = new RegExp(`(^|[^\\p{L}])(${w})(?=$|[^\\p{L}])`, 'giu');
  let any = false;
  for (const m of text.matchAll(re)) {
    any = true;
    const at = (m.index ?? 0) + m[1].length;
    const tok = m[2];
    const initial = /^\s*$|[.!?¿¡:]\s*$/.test(text.slice(0, at));
    if (!(/^\p{Lu}/u.test(tok) && !initial)) return false;
  }
  return any;
}

export function detectIdioms(text: string, lang: AciLang): Idiom[] {
  const list = IDIOMS[lang] ?? [];
  const found = list.filter((i) => hasPhrase(text, i.phrase));
  // keep the longest match when phrases overlap ("a piece of cake" ⊃ "piece of cake")
  return found.filter((i) => !found.some((j) => j !== i && j.phrase.includes(i.phrase)));
}

export interface ReferenceFinding {
  word: string;
  kind: 'place' | 'person';
  /** Exactly one plausible antecedent → it; 0 or 2+ → null (never guess). */
  antecedent: ContextEntity | null;
  candidates: number;
}

export function resolveReferences(text: string, lang: AciLang, ctx: SessionContext, opts: { topicBreak: boolean; currentEntities: ContextEntity[] }): ReferenceFinding[] {
  const refs = REFERENCES[lang];
  if (!refs) return [];
  const toks = words(text);
  const out: ReferenceFinding[] = [];
  const seen = new Set<string>();
  const latestSeq = ctx.lastTurn()?.seq ?? 0;
  for (const t of toks) {
    const kind = refs[t];
    if (!kind || seen.has(t)) continue;
    seen.add(t);
    // "there is/are", "allí hay" are existential, not references
    if (t === 'there' && /\bthere\s+(is|are|was|were|'s|will be|might|must|should)\b/i.test(text) && !/\b(go|going|get|be|meet|stay|wait|from|over|back|out)\s+there\b/i.test(text)) continue;
    const ok = (e: ContextEntity) => !e.supersededBy && (kind === 'place' ? e.kind === 'place' : e.kind === 'person' || e.kind === 'name');
    const pool = opts.topicBreak ? opts.currentEntities : [...ctx.entityList().filter((e) => latestSeq - e.lastSeq <= 4), ...opts.currentEntities];
    const distinct = new Map<string, ContextEntity>();
    for (const e of pool) if (ok(e)) distinct.set(e.text.toLowerCase(), e);
    // A name in this very utterance doesn't explain a pronoun that precedes it.
    out.push({ word: t, kind, antecedent: distinct.size === 1 ? [...distinct.values()][0] : null, candidates: distinct.size });
  }
  return out;
}
