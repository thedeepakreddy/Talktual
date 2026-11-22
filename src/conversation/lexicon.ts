// Small per-language word lists used by the turn/pause and self-repair
// heuristics. Deliberately conservative: a word is listed only if, at the END
// of a phrase, it almost always means the thought is unfinished (articles,
// prepositions, conjunctions, possessive determiners). Pronouns, auxiliaries
// and words like "that"/"then" are NOT listed: they often end real sentences
// ("how are you", "I think so", "I like that"). German separable-verb
// particles (an, auf, mit, zu) are excluded for the same reason. Languages not
// listed get no hold (recognizer finals pass straight through).

type Lex = { trailing: ReadonlySet<string>; repeatable: ReadonlySet<string>; corrections: readonly string[] };

const set = (s: string) => new Set(s.split(/\s+/).filter(Boolean));

const LEX: Record<string, Lex> = {
  en: {
    trailing: set('a an the to of for and or but with in on at from by into my your our their because if than whose'),
    repeatable: set('i a an the to of and or but in on at we you it is my'),
    corrections: ['i mean', 'sorry', 'no wait', 'wait', 'rather', 'or rather', 'actually'],
  },
  es: {
    trailing: set('a al de del el los las un una unos unas y o pero con en para que porque si mi tu su mis tus sus'),
    repeatable: set('a de el la los las un una y o en que yo mi'),
    corrections: ['digo', 'perdón', 'o sea', 'mejor dicho', 'espera'],
  },
  fr: {
    trailing: set('à au aux de du des un une et ou mais avec dans pour par que parce si mon ma mes ton ta son sa ses très'),
    repeatable: set('à de le la les un une et ou en que je'),
    corrections: ['je veux dire', 'pardon', 'enfin', 'attends'],
  },
  de: {
    trailing: set('ein eine einen einem eines und oder aber weil wenn dass für von zum zur mein meine dein deine'),
    repeatable: set('ein eine der die das und oder in zu ich'),
    corrections: ['ich meine', 'sorry', 'warte', 'also'],
  },
  pt: {
    trailing: set('o os as um uma de do da dos das e ou mas com em no na para que porque se meu minha teu seu sua'),
    repeatable: set('a o de do da e ou em que eu'),
    corrections: ['quer dizer', 'desculpa', 'ou seja', 'espera'],
  },
  it: {
    trailing: set('a al di del della il gli un una e o ma con nel per che perché se mio mia tuo suo sua'),
    repeatable: set('a di il lo la un una e o in che io'),
    corrections: ['cioè', 'scusa', 'voglio dire', 'aspetta'],
  },
};

/** Non-lexical hesitation sounds (uh, um, mm, hmm, erm, eh, ah …) in any language. */
const FILLER = /^(u+h+|u+m+|u+h+m+|e+r+m*|e+h+|a+h+|m{2,}|h+m+|e+m+|ehm+|euh+|äh+m*|öh+m*|ähm+)$/u;

export const baseLang = (lang: string) => lang.split('-')[0].toLowerCase();

export function lexicon(lang: string): Lex | null {
  return LEX[baseLang(lang)] ?? null;
}

export const isFiller = (word: string) => FILLER.test(word);

/** Lowercased word tokens without punctuation (keeps apostrophes/letters/digits). */
export function words(text: string): string[] {
  return text
    .toLowerCase()
    .replace(/[^\p{L}\p{N}' ]+/gu, ' ')
    .split(/\s+/)
    .filter(Boolean);
}
