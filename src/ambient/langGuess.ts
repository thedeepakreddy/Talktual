// Text-level language check for ambient mode (Phase 8).
//
// Web Speech needs the spoken language up front and has no audio language ID.
// When the room is speaking something else, the recognizer still returns
// words — forced into the chosen language. This check scores recognized text
// against small function-word lists and only reports a mismatch when the
// evidence is clear (enough words, clear margin). It never switches anything
// by itself; the UI offers the change. Covers the languages below only.

const STOP: Record<string, string[]> = {
  hu: ['a', 'az', 'és', 'hogy', 'nem', 'is', 'egy', 'van', 'meg', 'de', 'ez', 'csak', 'már', 'mert', 'igen', 'nincs', 'vagy', 'mi', 'én', 'te', 'ő', 'mit', 'hol', 'itt', 'ott', 'kell', 'még', 'akkor', 'nagyon', 'köszönöm', 'szia', 'jó', 'lesz', 'volt', 'vagyok', 'ezt', 'azt'],
  en: ['the', 'and', 'is', 'are', 'to', 'of', 'in', 'that', 'it', 'you', 'i', 'we', 'they', 'this', 'what', 'for', 'with', 'have', 'was', 'not', 'but', 'be', 'do', 'at', 'on', 'my', 'your', 'can', 'will', 'there'],
  es: ['el', 'la', 'de', 'que', 'y', 'en', 'los', 'las', 'es', 'no', 'por', 'un', 'una', 'con', 'para', 'del', 'se', 'lo', 'pero', 'muy', 'está', 'qué', 'yo', 'tú'],
  de: ['der', 'die', 'das', 'und', 'ist', 'nicht', 'ich', 'du', 'wir', 'sie', 'ein', 'eine', 'zu', 'mit', 'auf', 'den', 'dem', 'es', 'auch', 'aber', 'was', 'wie', 'noch', 'schon'],
  fr: ['le', 'la', 'les', 'et', 'est', 'de', 'des', 'un', 'une', 'je', 'tu', 'il', 'elle', 'nous', 'vous', 'que', 'qui', 'pas', 'ne', 'pour', 'dans', 'sur', 'avec', 'mais', 'oui', 'très'],
  it: ['il', 'la', 'di', 'che', 'e', 'è', 'non', 'un', 'una', 'per', 'con', 'sono', 'ma', 'io', 'tu', 'lui', 'lei', 'noi', 'questo', 'molto', 'anche', 'come', 'gli', 'del'],
  pt: ['o', 'a', 'os', 'as', 'de', 'que', 'e', 'é', 'não', 'um', 'uma', 'para', 'com', 'por', 'mas', 'eu', 'você', 'ele', 'ela', 'nós', 'muito', 'está', 'do', 'da'],
};
const SETS = Object.fromEntries(Object.entries(STOP).map(([k, v]) => [k, new Set(v)])) as Record<string, Set<string>>;

export const GUESSABLE = Object.keys(STOP);

export interface LangGuess {
  lang: string;
  /** Share of words that are function words of `lang`. */
  score: number;
  words: number;
}

export function guessLanguage(text: string): LangGuess | null {
  const ws = text
    .toLocaleLowerCase()
    .split(/[^\p{L}']+/u)
    .filter(Boolean);
  if (ws.length < 4) return null;
  let best: LangGuess | null = null;
  for (const [lang, set] of Object.entries(SETS)) {
    const hits = ws.filter((w) => set.has(w)).length;
    const score = hits / ws.length;
    if (!best || score > best.score) best = { lang, score, words: ws.length };
  }
  return best;
}

/**
 * Reports when the room has SWITCHED language: the trailing run of recognised
 * phrases that each clearly look like another language must total ≥ 12 words
 * (one ambiguous or expected-language phrase resets the run). It never acts.
 */
export class LanguageMismatchDetector {
  private run: { lang: string; words: number }[] = [];
  constructor(private expected: string) {}

  setExpected(lang: string): void {
    this.expected = lang;
    this.run = [];
  }

  /** Returns the base language code that the room seems to be speaking instead, or null. */
  add(text: string): string | null {
    const exp = this.expected.split('-')[0];
    if (!GUESSABLE.includes(exp)) return null;
    const ws = text.toLocaleLowerCase().split(/[^\p{L}']+/u).filter(Boolean);
    const g = guessLanguage(text);
    // Clear evidence for this phrase: another language explains ≥ 25 % of words and ≥ 2.5× the expected one.
    const other = g && g.lang !== exp && g.score >= 0.25 && g.score >= 2.5 * Math.max(scoreFor(text, exp), 0.01) ? g.lang : null;
    if (!other) {
      this.run = [];
      return null;
    }
    if (this.run.length && this.run[this.run.length - 1].lang !== other) this.run = [];
    this.run.push({ lang: other, words: ws.length });
    if (this.run.length > 6) this.run.shift();
    return this.run.reduce((n, r) => n + r.words, 0) >= 12 ? other : null;
  }

  reset(): void {
    this.run = [];
  }
}

function scoreFor(text: string, lang: string): number {
  const set = SETS[lang];
  const ws = text
    .toLocaleLowerCase()
    .split(/[^\p{L}']+/u)
    .filter(Boolean);
  return ws.length ? ws.filter((w) => set.has(w)).length / ws.length : 0;
}
