// Rule-based entity extraction: proper names, places, dates, times, numbers.
// Extractions are GUESSES with a confidence, never facts. Sensitive
// identifiers (phones, emails, card-like numbers) are redacted first and never
// become entities.

import { STOP, ADDRESS_WORDS, DAYS, MONTHS, NUMBER_WORDS, PERSON_CUES, PERSON_INTRO, PLACE_PREP, PLACE_WORDS } from './lexicon';
import { escapeRe, redactSensitive } from './text';
import type { AciLang, EntityKind } from './types';

export interface Extracted {
  text: string;
  kind: EntityKind;
  confidence: number;
  /** Should be kept verbatim through MT (person names, unknown names, street addresses). */
  protect: boolean;
}

/** Common words that start sentences (imperatives, function words) — never the first word of a name. */
const COMMON_INITIAL = new Set([
  ...STOP,
  ...'tell ask call send bring take wait come give show help meet pick text email write remind let thank see find book check say dile pregúntale llama manda trae dame ayuda espera ven mira dime pásale avísale saluda'.split(' '),
]);

/** Frequent sentence openers that are not names (kept small and reviewable). */
const COMMON_STARTERS = new Set(
  'maybe perhaps today tonight tomorrow yesterday then first next finally also actually honestly anyway unfortunately luckily hopefully basically really sorry thanks hello please everything nothing someone somebody everyone nobody there here where when what which who why how can could would should will shall does did is are was were have has had let lets let\'s just only even still yet again quizás quizá tal hoy mañana ayer luego después antes primero entonces también todavía siempre nunca gracias perdón ojalá creo dice dijo vamos tengo tenemos hay está estoy estamos son era fue puede podemos quiero necesito llegamos llego voy fuimos'.split(
    ' '
  )
);

const CONNECT = new Set(['de', 'del', 'la', 'las', 'los', 'of', 'the', 'y', 'and', 'van', 'von', 'da', 'di']);
const NOT_NAMES = new Set(
  'I I’m I\'m OK Okay Yes No Hello Hi Hey Thanks Thank Please Sorry Well So And But Then Oh Yeah Sure Hola Gracias Vale Sí Bueno Pues Perdón Perdona Oye Mira Claro Buenos Buenas Adiós Listen Look Wait Great Good Right'.split(
    ' '
  )
);

/** Capitalized spans (2+ tokens may include connectors like "de", "of"). */
function capitalSpans(text: string): Array<{ text: string; start: number; initial: boolean }> {
  const out: Array<{ text: string; start: number; initial: boolean }> = [];
  const re = /[\p{L}][\p{L}'’.-]*/gu;
  const toks: Array<{ w: string; i: number }> = [];
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) toks.push({ w: m[0], i: m.index });
  const isCap = (w: string) => /^\p{Lu}/u.test(w);
  const sentenceInitial = (i: number) => {
    const before = text.slice(0, i).trimEnd();
    return before === '' || /[.!?¿¡:]$/.test(before);
  };
  for (let k = 0; k < toks.length; k++) {
    if (!isCap(toks[k].w) || NOT_NAMES.has(toks[k].w.replace(/[.’']+$/, ''))) continue;
    let end = k;
    while (end + 1 < toks.length) {
      if (isCap(toks[end + 1].w) && !NOT_NAMES.has(toks[end + 1].w)) end++;
      else if (end + 2 < toks.length && CONNECT.has(toks[end + 1].w.toLowerCase()) && isCap(toks[end + 2].w)) end += 2;
      else break;
    }
    const start = toks[k].i;
    const last = toks[end];
    out.push({ text: text.slice(start, last.i + last.w.length).replace(/[.]+$/, ''), start, initial: sentenceInitial(start) });
    k = end;
  }
  return out;
}

export function extractEntities(raw: string, lang: AciLang, known: (t: string) => boolean = () => false): Extracted[] {
  const { text } = redactSensitive(raw);
  const out: Extracted[] = [];
  const seen = new Set<string>();
  const push = (e: Extracted) => {
    const k = `${e.kind}:${e.text.toLowerCase()}`;
    if (seen.has(k) || !e.text.trim()) return;
    seen.add(k);
    out.push(e);
  };
  const lower = text.toLowerCase();

  for (let span of capitalSpans(text)) {
    // "Tell Rose …": a common word capitalized only because it starts the sentence is not part of the name.
    const head = span.text.split(/\s+/)[0];
    if (span.initial && span.text.includes(' ') && COMMON_INITIAL.has(head.toLowerCase()) && !known(span.text)) {
      const rest = span.text.slice(head.length).trimStart();
      span = { text: rest, start: span.start + span.text.indexOf(rest), initial: false };
    }
    const tokens = span.text.split(/\s+/);
    const multi = tokens.length > 1;
    // A lone capitalized word at the start of a sentence is usually just grammar.
    // Exception: an uncommon word followed by lowercase text ("Ana viene…") is kept
    // as a LOW-confidence name — never protected, only used to stop pronoun guesses.
    if (span.initial && !multi && !known(span.text)) {
      const w = span.text.toLowerCase();
      const isDate = [...(DAYS[lang] ?? []), ...(MONTHS[lang] ?? [])].includes(w);
      if (!COMMON_INITIAL.has(w) && !COMMON_STARTERS.has(w) && !isDate && w.length >= 3) push({ text: span.text, kind: 'name', confidence: 0.4, protect: false });
      continue;
    }
    const before = text.slice(0, span.start).toLowerCase();
    const prevWord = before.trim().split(/\s+/).at(-1)?.replace(/[.,]/g, '') ?? '';
    const lowTokens = tokens.map((t) => t.toLowerCase().replace(/[.,]/g, ''));
    let kind: EntityKind = 'name';
    let confidence = 0.6;
    if (lowTokens.some((t) => PLACE_WORDS.has(t))) {
      kind = 'place';
      confidence = 0.85;
    } else if (PERSON_CUES.has(prevWord) || PERSON_CUES.has(lowTokens[0])) {
      kind = 'person';
      confidence = 0.85;
    } else if ((PLACE_PREP[lang] ?? []).some((p) => new RegExp(`(^|\\s)${escapeRe(p)}\\s*$`).test(before))) {
      // "in Madrid", "a Sevilla" — places; but "with Rose" is handled below
      kind = 'place';
      confidence = 0.65;
    }
    if (kind !== 'place' && PERSON_INTRO[lang]?.test(before)) {
      kind = 'person';
      confidence = Math.max(confidence, 0.7);
    }
    const protect = kind === 'person' || kind === 'name' || (kind === 'place' && lowTokens.some((t) => ADDRESS_WORDS.has(t)));
    push({ text: span.text, kind, confidence, protect });
  }

  // Times: 5pm, 5 pm, 17:30, 5 o'clock, a las cinco
  for (const m of text.matchAll(/\b(\d{1,2}(:\d{2})?\s?(am|pm|a\.m\.|p\.m\.|h)\b|\d{1,2}:\d{2}\b|\d{1,2} o'clock)/gi)) push({ text: m[0], kind: 'time', confidence: 0.9, protect: false });
  for (const m of lower.matchAll(/\b(a las|at) ((\d{1,2}|una|uno|dos|tres|cuatro|cinco|seis|siete|ocho|nueve|diez|once|doce|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve)( y media| y cuarto| menos cuarto|:\d{2}| thirty| fifteen)?)\b/g)) {
    if (out.some((e) => e.kind === 'time' && e.text.toLowerCase().startsWith(m[2]))) continue;
    push({ text: m[2], kind: 'time', confidence: 0.75, protect: false });
  }
  // Dates: weekdays, months (+ day numbers)
  for (const d of [...(DAYS[lang] ?? []), ...(MONTHS[lang] ?? [])]) {
    const re = new RegExp(`(^|[^\\p{L}])(${escapeRe(d)}(\\s+\\d{1,2}(st|nd|rd|th)?)?|\\d{1,2}(\\s+de)?\\s+${escapeRe(d)})(?=$|[^\\p{L}])`, 'u');
    const m = lower.match(re);
    if (m) push({ text: m[2], kind: 'date', confidence: 0.85, protect: false });
  }
  // Plain numbers / amounts (short; long digit runs were redacted)
  for (const m of text.matchAll(/(?<![\p{L}\d:])(\d{1,5}([.,]\d{1,2})?)\s?(€|euros?|dollars?|\$|%|km|kilometers|kilómetros|minutes|minutos|hours|horas|people|personas)?(?![\d:])/gu)) {
    if (out.some((e) => e.kind === 'time' && e.text.includes(m[1]))) continue;
    push({ text: m[0].trim(), kind: 'number', confidence: 0.8, protect: false });
  }
  for (const w of NUMBER_WORDS[lang] ?? []) {
    const m = lower.match(new RegExp(`(^|\\s)(${escapeRe(w)})\\s+(people|personas|euros|dollars|minutes|minutos|hours|horas|nights|noches|tickets|entradas|rooms|habitaciones)\\b`));
    if (m) push({ text: `${m[2]} ${m[3]}`, kind: 'number', confidence: 0.7, protect: false });
  }
  return out;
}
