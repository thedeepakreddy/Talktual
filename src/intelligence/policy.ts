// Decisions and their validation. `decide*` proposes; `validate*` enforces the
// invariants the session relies on. An invalid decision is replaced by the
// faithful, context-free fallback — never partially applied.

import { FILLERS, type AmbiguousWord } from './lexicon';
import { buildMtInput } from './protect';
import { escapeRe, words } from './text';
import { ACI_VERSION, type AciLang, type AciLimits, type InboundDecision, type Note, type OutboundDecision, type Signal } from './types';

const ASSIST_CODE = /^(sense:[\p{L}'-]{1,30}:[a-z-]{1,20}|protect:\d{1,2})$/u;
const MAX_TEXT = 220;

export function faithfulOutbound(segmentId: string, text: string, fallback?: OutboundDecision['fallback']): OutboundDecision {
  return { v: ACI_VERSION, type: 'outbound', segmentId, mtInput: text, protections: [], edits: [], assist: [], signals: [], notes: [], suggestions: [], ...(fallback ? { fallback } : {}) };
}
export function faithfulInbound(segmentId: string, fallback?: InboundDecision['fallback']): InboundDecision {
  return { v: ACI_VERSION, type: 'inbound', segmentId, notes: [], signals: [], suggestions: [], ...(fallback ? { fallback } : {}) };
}

const validSignals = (s: Signal[]) => s.length <= 12 && s.every((x) => x.confidence >= 0 && x.confidence <= 1);
const validNotes = (n: Note[], limits: AciLimits) => n.length <= limits.maxNotesPerSegment && n.every((x) => x.text.length > 0 && x.text.length <= MAX_TEXT);

export function validateOutbound(d: OutboundDecision, original: string, lexicon: AmbiguousWord[] | undefined, limits: AciLimits): boolean {
  if (d.v !== ACI_VERSION || d.type !== 'outbound') return false;
  // Every protected value really occurs in what the speaker said; tokens are unique.
  if (d.protections.length > 16) return false;
  if (new Set(d.protections.map((p) => p.token)).size !== d.protections.length) return false;
  if (!d.protections.every((p) => p.value.length > 0 && original.includes(p.value))) return false;
  // Every edit is a lexicon sense edit of a word that occurs in the original.
  for (const e of d.edits) {
    const entry = lexicon?.find((w) => w.word === e.word || w.forms?.includes(e.word));
    const sense = entry?.senses.find((s) => s.id === e.senseId);
    if (!sense?.edit || sense.edit !== e.replacement) return false;
    if (!words(original).includes(e.word)) return false;
  }
  // The MT input is exactly the deterministic construction — no other change, no added content.
  if (d.mtInput !== buildMtInput(original, d.protections, d.edits)) return false;
  if (d.assist.length > 8 || !d.assist.every((a) => ASSIST_CODE.test(a))) return false;
  return validSignals(d.signals) && validNotes(d.notes, limits) && d.suggestions.length <= 2;
}

/** Concise delivery: remove closed-list discourse fillers only. */
export function concise(text: string, lang: AciLang): { text: string; removed: number } {
  let out = text;
  let removed = 0;
  for (const f of FILLERS[lang] ?? []) {
    const bare = f.replace(/,$/, '');
    const re = f.endsWith(',')
      ? new RegExp(`(^|[.!?¿¡]\\s*)${escapeRe(bare)},\\s*`, 'giu') // "well, …" only as a sentence opener
      : new RegExp(`(^|[\\s,])${escapeRe(bare)}(,)?(?=$|[\\s,.!?])`, 'giu');
    out = out.replace(re, (_m, pre) => {
      removed++;
      return pre;
    });
  }
  out = out.replace(/\s{2,}/g, ' ').replace(/\s+([,.!?])/g, '$1').replace(/^[,\s]+/, '').trim();
  if (out && /^\p{Lu}/u.test(text.trim())) out = out[0].toUpperCase() + out.slice(1);
  return { text: out, removed };
}

export function validateInbound(d: InboundDecision, translated: string, limits: AciLimits): boolean {
  if (d.v !== ACI_VERSION || d.type !== 'inbound') return false;
  if (d.speak) {
    // Concise output must be a subsequence of the faithful words (nothing reordered or added).
    const a = words(translated);
    const b = words(d.speak.text);
    let i = 0;
    for (const w of a) if (w === b[i]) i++;
    if (i !== b.length || b.length === 0) return false;
    // Bounded removal: at most half the words, and at least two words remain.
    if (b.length < a.length * 0.5 || b.length < 2) return false;
  }
  return validSignals(d.signals) && validNotes(d.notes, limits) && d.suggestions.length <= 2;
}
