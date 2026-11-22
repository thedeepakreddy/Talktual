// Offline phrasebook translator (English ↔ Spanish), on-device, no model.
//
// Honest by construction: an utterance is translated only if EVERY part of it
// is covered by a phrase or a slot template; otherwise the result is "not
// covered" (null) — never a partial translation that silently drops words.

import { ITEMS, NUMBERS, PHRASES, PLACES, type PhraseEntry, type SlotValue } from './phrasebookData';
import { base, LOCAL_RUNTIME_VERSION, type LocalTranslateRequest, type LocalTranslateResult, type Readiness, type RuntimeInfo, type TranslateRuntime } from './types';

type Lang = 'en' | 'es';

export function fold(s: string): string {
  return s
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[’`]/g, "'")
    .replace(/[^a-z0-9'ñ\s:]/g, ' ') // ñ survives NFD as n+~ → folded to n above; kept for safety
    .replace(/\s+/g, ' ')
    .trim();
}

const FILLERS: Record<Lang, string[]> = {
  en: ['um', 'uh', 'er', 'erm', 'hmm', 'so', 'well', 'oh', 'like'],
  es: ['eh', 'em', 'pues', 'bueno', 'este', 'mmm', 'oye', 'mira'],
};
const PLEASE: Record<Lang, string[]> = { en: ['please'], es: ['por favor'] };
const EXCUSE: Record<Lang, string[]> = { en: ['excuse me'], es: ['disculpe', 'perdone', 'perdona', 'disculpa'] };

interface Template {
  en: string[];
  es: string[];
  q?: boolean;
}
const T = (en: string[], es: string[], q = false): Template => ({ en, es, q });

const TEMPLATES: Template[] = [
  // Every variant inside one template means the SAME thing (output uses the first
  // target variant with the same slots). Variants that differ in meaning
  // ("we" vs "I", "tomorrow", "nearest") are separate templates.
  T(['where is the {place}', 'where is {placeI}', "where's the {place}"], ['dónde está {place}', 'dónde queda {place}'], true),
  T(['how do i get to the {place}', 'how can i get to the {place}', 'how do i get to {placeI}'], ['cómo llego {placeA}', 'cómo puedo llegar {placeA}', 'cómo se llega {placeA}'], true),
  T(['how do we get to the {place}', 'how can we get to the {place}'], ['cómo llegamos {placeA}', 'cómo podemos llegar {placeA}'], true),
  T(['is there {placeI} near here', 'is there {placeI} nearby', 'is there {placeI} around here'], ['hay {placeI} cerca', 'hay {placeI} por aquí', 'hay {placeI} cerca de aquí'], true),
  T(['take me to the {place}', 'can you take me to the {place}'], ['lléveme {placeA}', 'llévame {placeA}', 'me lleva {placeA}', 'me puede llevar {placeA}']),
  T(["i'm at the {place}", 'i am at the {place}'], ['estoy en {place}']),
  T(["we're at the {place}", 'we are at the {place}'], ['estamos en {place}']),
  T(['meet me at the {place}', "let's meet at the {place}", 'see you at the {place}'], ['nos vemos en {place}', 'quedamos en {place}']),
  T(['i need {item}'], ['necesito {item}']),
  T(['we need {item}'], ['necesitamos {item}']),
  T(["i'd like {item}", 'i would like {item}', 'can i have {item}', 'could i have {item}', 'can i get {item}'], ['quisiera {item}', 'me gustaría {item}', 'me pone {item}', 'me da {item}', 'me pones {item}', 'quiero {item}']),
  T(["we'd like {item}", 'we would like {item}', 'can we have {item}'], ['queremos {item}', 'nos gustaría {item}', 'nos pone {item}']),
  T(['do you have {item}', 'have you got {item}'], ['tiene {item}', 'tienes {item}', 'tienen {item}'], true),
  T(['how much is {item}', 'how much does {item} cost', 'how much for {item}'], ['cuánto cuesta {item}', 'cuánto vale {item}', 'cuánto es {item}'], true),
  T(['where can i buy {item}'], ['dónde puedo comprar {item}'], true),
  T(['{n} tickets'], ['{n} billetes', '{n} entradas']),
  T(['{n} tickets to the {place}'], ['{n} billetes para {place}']),
  T(['a table for {n}', 'table for {n}', 'a table for {n} people'], ['una mesa para {n}', 'mesa para {n}', 'una mesa para {n} personas']),
  T(['a room for {n} nights'], ['una habitación para {n} noches']),
  T(['a room for {n} people'], ['una habitación para {n} personas']),
  T(['{n} people', 'for {n} people'], ['{n} personas', 'para {n} personas']),
  T(['we are {n}', "we're {n}", 'there are {n} of us'], ['somos {n}']),
  T(['{n} minutes'], ['{n} minutos']),
  T(['in {n} minutes'], ['en {n} minutos']),
  T(['at {time}'], ['a las {time}', 'a la {time}']),
  T(["it's {time}", 'it is {time}'], ['son las {time}', 'es la {time}']),
  T(['see you at {time}'], ['nos vemos a las {time}']),
  T(['see you tomorrow at {time}'], ['nos vemos mañana a las {time}', 'hasta mañana a las {time}']),
  T(['the train leaves at {time}'], ['el tren sale a las {time}']),
  T(['the bus leaves at {time}'], ['el autobús sale a las {time}']),
  T(['it costs {n} euros', 'that costs {n} euros'], ['cuesta {n} euros', 'vale {n} euros']),
  T(["it's {n} euros", 'it is {n} euros', "that's {n} euros", 'that is {n} euros'], ['son {n} euros', 'es {n} euros']),
  T(['{n} euros'], ['{n} euros']),
  T(['my name is {name}'], ['me llamo {name}', 'mi nombre es {name}']),
];

// ---- slot forms ----------------------------------------------------------------
const article = (en: string) => (/^[aeiou]/.test(en) ? `an ${en}` : `a ${en}`);
const esA = (def: string) => def.replace(/^el /, 'al ').replace(/^la /, 'a la ').replace(/^los /, 'a los ').replace(/^las /, 'a las ').replace(/^(?!al |a )/, 'a ');

interface SlotForm {
  key: string;
  out: { en: string; es: string };
}
function slotTable(kind: string): Record<Lang, Array<{ match: string; out: { en: string; es: string } }>> {
  const t: Record<Lang, Array<{ match: string; out: { en: string; es: string } }>> = { en: [], es: [] };
  const add = (vals: SlotValue[], enF: (v: string) => string, esF: (v: SlotValue) => string[], esOut: (v: SlotValue) => string) => {
    for (const v of vals) {
      const out = { en: enF(v.en[0]), es: esOut(v) };
      for (const e of v.en) t.en.push({ match: fold(enF(e)), out });
      for (const s of esF(v)) t.es.push({ match: fold(s), out });
    }
  };
  if (kind === 'place') add(PLACES, (e) => e, (v) => v.esDef, (v) => v.esDef[0]);
  if (kind === 'placeI') add(PLACES.filter((p) => p.esIndef), article, (v) => v.esIndef!, (v) => v.esIndef![0]);
  if (kind === 'placeA') add(PLACES, (e) => e, (v) => v.esDef.map(esA), (v) => esA(v.esDef[0]));
  if (kind === 'item') add(ITEMS, (e) => e, (v) => v.esDef, (v) => v.esDef[0]);
  // longest first so "train station" beats "station"
  t.en.sort((a, b) => b.match.length - a.match.length);
  t.es.sort((a, b) => b.match.length - a.match.length);
  return t;
}

const NUM_EN = new Map(NUMBERS.map(([e, s]) => [fold(e), s]));
const NUM_ES = new Map(NUMBERS.map(([e, s]) => [fold(s), e]));
function numberSlot(raw: string, from: Lang): string | null {
  const t = fold(raw);
  if (/^\d{1,4}([.,]\d{1,2})?$/.test(t)) return raw.trim();
  const m = from === 'en' ? NUM_EN.get(t) : NUM_ES.get(t);
  return m ?? null;
}

const TIME_EXTRA: Array<[string, string]> = [
  ["o'clock", ''],
  ['oclock', ''],
  ['thirty', 'y media'],
  ['fifteen', 'y cuarto'],
  ['am', 'de la mañana'],
  ['pm', 'de la tarde'],
  ['in the morning', 'de la mañana'],
  ['in the afternoon', 'de la tarde'],
  ['in the evening', 'de la tarde'],
  ['at night', 'de la noche'],
];
function timeSlot(raw: string, from: Lang): string | null {
  const t = fold(raw);
  const hm = t.match(/^(\d{1,2})(:(\d{2}))?$/);
  if (hm && Number(hm[1]) <= 24) return t;
  const toks = t.split(' ');
  const n = numberSlot(toks[0], from);
  if (!n || (/^\d+$/.test(n) ? Number(n) : 1) > 24) return null;
  const rest = toks.slice(1).join(' ');
  if (!rest) return n;
  const pair = TIME_EXTRA.find(([en, es]) => (from === 'en' ? fold(en) === rest : es && fold(es) === rest));
  if (!pair) return null;
  const extra = from === 'en' ? pair[1] : pair[0];
  return extra ? `${n} ${extra}` : n;
}

/** Name slot: 1–3 tokens, copied verbatim from the ORIGINAL text (case preserved), or a protection token. */
const NAME_RE = /^([a-z][a-z'-]{0,30}|qx\d+q)( [a-z][a-z'-]{0,30}){0,2}$/;

// ---- compiled matcher -----------------------------------------------------------
interface Compiled {
  re: RegExp;
  slots: string[];
  tpl: Template;
}
const SLOT_NAMES: Record<string, string> = { placei: 'placeI', placea: 'placeA' };
/** {placeA} renders like {place} in English, so they are interchangeable for output selection. */
const canonSlot = (s: string) => (s === 'placeA' ? 'place' : s);
const slotsOf = (pattern: string) => [...pattern.matchAll(/\{(\w+)\}/g)].map((m) => canonSlot(m[1])).sort().join(',');

function compile(pattern: string): { re: RegExp; slots: string[] } {
  const slots: string[] = [];
  const src = fold(pattern.replace(/\{(\w+)\}/g, ' SLOT$1SLOT '))
    .split(' ')
    .map((tok) => {
      const m = tok.match(/^slot(\w+)slot$/);
      if (m) {
        slots.push(SLOT_NAMES[m[1]] ?? m[1]);
        return '(.+?)';
      }
      return tok.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    })
    .join(' ');
  return { re: new RegExp(`^${src}$`), slots };
}

export class Phrasebook {
  private exact: Record<Lang, Map<string, PhraseEntry>> | null = null;
  private templates: Record<Lang, Compiled[]> | null = null;
  private slots: Record<string, ReturnType<typeof slotTable>> = {};
  loadMs: number | null = null;

  load(now: () => number = () => Date.now()): void {
    if (this.exact) return;
    const t0 = now();
    const exact: Record<Lang, Map<string, PhraseEntry>> = { en: new Map(), es: new Map() };
    for (const p of PHRASES) {
      for (const e of p.en) exact.en.set(fold(e), p);
      for (const s of p.es) exact.es.set(fold(s), p);
    }
    const templates: Record<Lang, Compiled[]> = { en: [], es: [] };
    for (const tpl of TEMPLATES) {
      for (const l of ['en', 'es'] as const) {
        for (const pat of tpl[l]) {
          const c = compile(pat);
          templates[l].push({ ...c, tpl });
        }
      }
    }
    for (const k of ['place', 'placeI', 'placeA', 'item']) this.slots[k] = slotTable(k);
    this.exact = exact;
    this.templates = templates;
    this.loadMs = now() - t0;
  }

  unload(): void {
    this.exact = null;
    this.templates = null;
    this.slots = {};
  }

  get loaded(): boolean {
    return this.exact !== null;
  }

  /** Translate or return null (not covered). */
  translate(text: string, from: Lang, to: Lang): LocalTranslateResult {
    if (!this.exact) this.load();
    if (from === to || text.length > 300 || !(['en', 'es'] as string[]).includes(from) || !(['en', 'es'] as string[]).includes(to)) return null;
    const original = text.trim();
    if (!original) return null;
    const whole = this.unit(original, from);
    if (whole) return { text: whole.text, coverage: whole.coverage };
    // Sentence / clause split: every part must be covered.
    const parts = original.split(/(?<=[.!?;,])\s+|\s+[—–-]\s+/).filter((p) => fold(p));
    if (parts.length > 1 && parts.length <= 4) {
      const outs = parts.map((p) => this.unit(p, from));
      if (outs.every(Boolean)) return { text: outs.map((o) => o!.text).reduce((x, y) => joinClauses(x, y)), coverage: outs.some((o) => o!.coverage === 'template') ? 'template' : 'exact' };
    }
    // Unpunctuated speech: try one split point ("hello where is the station").
    const toks = original.split(/\s+/);
    if (toks.length >= 2 && toks.length <= 20) {
      for (let i = 1; i < toks.length; i++) {
        const a = this.unit(toks.slice(0, i).join(' '), from);
        if (!a) continue;
        const b = this.unit(toks.slice(i).join(' '), from);
        if (b) return { text: joinClauses(a.text, b.text), coverage: a.coverage === 'template' || b.coverage === 'template' ? 'template' : 'exact' };
      }
    }
    return null;
  }

  /** One clause, with optional leading filler, leading "excuse me", leading/trailing "please". */
  private unit(raw: string, from: Lang): { text: string; coverage: 'exact' | 'template' } | null {
    const to: Lang = from === 'en' ? 'es' : 'en';
    let origToks = raw.trim().split(/\s+/).filter((t) => fold(t));
    let toks = origToks.map(fold);
    const isQ = /\?\s*$|^¿/.test(raw.trim());
    // leading fillers (only if something remains)
    while (toks.length > 1 && FILLERS[from].includes(toks[0])) {
      toks = toks.slice(1);
      origToks = origToks.slice(1);
    }
    let excuse = false;
    for (const e of EXCUSE[from]) {
      const n = e.split(' ').length;
      if (toks.length > n && toks.slice(0, n).join(' ') === e) {
        toks = toks.slice(n);
        origToks = origToks.slice(n);
        excuse = true;
      }
    }
    let please = false;
    for (const pl of PLEASE[from]) {
      const n = pl.split(' ').length;
      if (toks.length > n && toks.slice(-n).join(' ') === pl) {
        toks = toks.slice(0, -n);
        origToks = origToks.slice(0, -n);
        please = true;
      } else if (toks.length > n && toks.slice(0, n).join(' ') === pl) {
        toks = toks.slice(n);
        origToks = origToks.slice(n);
        please = true;
      }
    }
    const key = toks.join(' ');
    if (!key) return null;
    let core: { text: string; q: boolean; coverage: 'exact' | 'template' } | null = null;
    const hit = this.exact![from].get(key);
    if (hit) core = { text: hit[to][0], q: Boolean(hit.q), coverage: 'exact' };
    else core = this.template(key, origToks, from);
    if (!core) return null;
    let out = core.text;
    if (please && !fold(out).includes(fold(PLEASE[to][0]))) out = `${out}, ${PLEASE[to][0]}`;
    let formatted = format(out, to, core.q || isQ);
    if (excuse) formatted = `${to === 'es' ? 'Disculpe' : 'Excuse me'}, ${lowerFirst(formatted)}`;
    return { text: formatted, coverage: core.coverage };
  }

  private template(key: string, origToks: string[], from: Lang): { text: string; q: boolean; coverage: 'template' } | null {
    const to: Lang = from === 'en' ? 'es' : 'en';
    for (const c of this.templates![from]) {
      const m = key.match(c.re);
      if (!m) continue;
      const fills: Record<string, string> = {};
      let ok = true;
      c.slots.forEach((slot, i) => {
        if (!ok) return;
        const captured = m[i + 1];
        const v = this.fill(slot, captured, from, to, key, origToks);
        if (v === null) ok = false;
        else fills[canonSlot(slot)] = v;
      });
      if (!ok) continue;
      // Output: the first target pattern with EXACTLY the same slots (no slot is ever dropped).
      const want = c.slots.map(canonSlot).sort().join(',');
      const outPattern = c.tpl[to].find((pt) => slotsOf(pt) === want);
      if (!outPattern) continue;
      // placeA in Spanish output needs the "a"/"al" form of the place
      let out = outPattern.replace(/\{(\w+)\}/g, (_m, name: string) => {
        const v = fills[canonSlot(name)];
        return name === 'placeA' && to === 'es' ? esA(v) : v;
      });
      if (to === 'es') out = out.replace(/\ba las (una|1)\b/, 'a la $1').replace(/\bson las (una|1)\b/, 'es la $1');
      return { text: out, q: Boolean(c.tpl.q), coverage: 'template' };
    }
    return null;
  }

  private fill(slot: string, captured: string, from: Lang, to: Lang, key: string, origToks: string[]): string | null {
    if (slot === 'n') return numberSlot(captured, from);
    if (slot === 'time') return timeSlot(captured, from);
    if (slot === 'name') {
      if (!NAME_RE.test(captured)) return null;
      // recover original casing: find the captured token run in the folded key
      const capToks = captured.split(' ');
      const keyToks = key.split(' ');
      for (let i = 0; i + capToks.length <= keyToks.length; i++) {
        if (keyToks.slice(i, i + capToks.length).join(' ') === captured) {
          const offset = origToks.length - keyToks.length;
          return origToks.slice(i + offset, i + offset + capToks.length).join(' ').replace(/[.,!?¿¡]+$/, '');
        }
      }
      return null;
    }
    const table = this.slots[slot]?.[from];
    const hit = table?.find((v) => v.match === captured);
    return hit ? hit.out[to] : null;
  }
}

const lowerFirst = (t: string) => t.replace(/^(¿?)(\p{Lu})/u, (_m, q: string, c: string) => q + c.toLowerCase());
/** "Hola" + "¿Dónde está…?" → "Hola, ¿dónde está…?"; after a sentence end, keep the capital. */
function joinClauses(a: string, b: string): string {
  return /[.?!]$/.test(a) ? `${a} ${b}` : `${a}, ${lowerFirst(b)}`;
}

function format(text: string, lang: Lang, question: boolean): string {
  let t = text.trim().replace(/\s+,/g, ',');
  if (lang === 'en') t = t.replace(/\bi\b/g, 'I').replace(/\bi'(m|d|ll|ve)\b/g, "I'$1").replace(/\batm\b/g, 'ATM').replace(/\bwifi\b/g, 'Wi-Fi');
  t = t.charAt(0).toUpperCase() + t.slice(1);
  if (question) {
    t = t.replace(/[?¿]/g, '');
    t = lang === 'es' ? `¿${t}?` : `${t}?`;
  }
  return t;
}

// ---- runtime adapter -------------------------------------------------------------

export class PhrasebookRuntime implements TranslateRuntime {
  readonly id = 'phrasebook-en-es';
  private pb = new Phrasebook();
  constructor(private readonly now: () => number = () => Date.now()) {}

  info(): RuntimeInfo {
    return {
      v: LOCAL_RUNTIME_VERSION,
      id: this.id,
      task: 'translate',
      languages: ['en>es', 'es>en'],
      model: { name: 'Talktual offline phrasebook', version: '1', license: 'project (original content)', sizeKB: 22, install: 'bundled with the app' },
      platform: 'any browser (pure JavaScript)',
      readiness: 'ready',
      constraints: ['Common travel and everyday phrases only', 'Whole utterance must be covered — otherwise it is left untranslated', 'English ↔ Spanish'],
      quality: 'Exact for covered phrases; no translation at all outside coverage',
      resources: { memoryKB: null, loadMs: this.pb.loadMs },
      verification: 'verified-test-hardware',
    };
  }
  async discover(): Promise<Readiness> {
    return 'ready';
  }
  supports(from: string, to: string): boolean {
    const f = base(from);
    const t = base(to);
    return (f === 'en' && t === 'es') || (f === 'es' && t === 'en');
  }
  async load(): Promise<void> {
    this.pb.load(this.now);
  }
  unload(): void {
    this.pb.unload();
  }
  async translate(req: LocalTranslateRequest): Promise<LocalTranslateResult> {
    if (!this.supports(req.from, req.to)) return null;
    return this.pb.translate(req.text, base(req.from) as Lang, base(req.to) as Lang);
  }
  dispose(): void {
    this.pb.unload();
  }
}
