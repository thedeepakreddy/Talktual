// Self-repair and completeness heuristics. Pure functions on text that has
// NOT yet been spoken to the listener; nothing here ever touches a unit after
// it has been committed.

import { isFiller, lexicon, words } from './lexicon';

/** Only hesitation sounds (or nothing). */
export function isFillerOnly(text: string): boolean {
  const w = words(text);
  return w.length === 0 || w.every(isFiller);
}

/**
 * Does the phrase look unfinished? True when it ends in a word that almost
 * never ends a thought ("to", "the", "and"…), a hesitation sound, or a comma.
 * Terminal punctuation means complete.
 */
export function looksIncomplete(text: string, lang: string): boolean {
  const t = text.trim();
  if (!t) return false;
  if (/[.?!。？！]$/.test(t)) return false;
  if (/[,;:\-–]$/.test(t)) return true;
  const w = words(t);
  const last = w.at(-1);
  if (!last) return false;
  if (isFiller(last)) return true;
  const lex = lexicon(lang);
  return Boolean(lex?.trailing.has(last));
}

/**
 * Removes immediate repetitions: repeated 3- and 2-word sequences ("can you can
 * you call" → "can you call") and repeated single FUNCTION words ("the the").
 * Repeated content words are kept ("very very", "no no") — they are often
 * deliberate. Also strips leading hesitation sounds.
 */
export function collapseRepetitions(text: string, lang: string): { text: string; repaired: boolean } {
  const orig = text.trim().split(/\s+/).filter(Boolean);
  let toks = [...orig];
  const key = (s: string) => words(s).join(' ');
  // leading fillers
  while (toks.length > 1 && words(toks[0]).every(isFiller)) toks.shift();
  for (const n of [3, 2]) {
    for (let i = 0; i + 2 * n <= toks.length; ) {
      const a = toks.slice(i, i + n).map(key).join(' ');
      const b = toks.slice(i + n, i + 2 * n).map(key).join(' ');
      // A run of ONE repeated word is a 1-gram case (handled below), not a phrase.
      const oneWord = new Set(toks.slice(i, i + n).map(key)).size === 1;
      if (a && a === b && !oneWord) toks.splice(i + n, n);
      else i++;
    }
  }
  const rep = lexicon(lang)?.repeatable;
  if (rep) {
    for (let i = 0; i + 1 < toks.length; ) {
      const a = key(toks[i]);
      if (a && a === key(toks[i + 1]) && rep.has(a)) toks.splice(i + 1, 1);
      else i++;
    }
  }
  return { text: toks.join(' '), repaired: toks.length !== orig.length };
}

export type Restart = { kind: 'replace'; text: string } | { kind: 'merge' };

/**
 * A new final arrives while an incomplete fragment is held (not yet spoken).
 * Decide whether the speaker RESTARTED (the new text supersedes the fragment)
 * or CONTINUED (merge):
 *   * new text begins with the whole held fragment  → restart ("i want to" → "i want to go…")
 *   * new text begins with the same first two words → restart
 *   * new text begins with a correction marker ("I mean", "sorry") → the
 *     remainder replaces the fragment
 */
export function resolveRestart(held: string, next: string, lang: string): Restart {
  const h = words(held);
  const n = words(next);
  if (!h.length || !n.length) return { kind: 'merge' };
  const lex = lexicon(lang);
  if (lex) {
    const joined = n.join(' ');
    for (const marker of lex.corrections) {
      if (joined === marker) return { kind: 'merge' };
      if (joined.startsWith(marker + ' ')) {
        const markerWords = marker.split(' ').length;
        const rest = next.trim().split(/\s+/).slice(markerWords).join(' ').replace(/^[,;:\s]+/, '');
        if (rest) return { kind: 'replace', text: rest };
      }
    }
  }
  if (n.length >= h.length && h.every((w, i) => n[i] === w)) return { kind: 'replace', text: next.trim() };
  if (h.length >= 2 && n.length >= 2 && h[0] === n[0] && h[1] === n[1]) return { kind: 'replace', text: next.trim() };
  return { kind: 'merge' };
}
