// Phase 5 ACI evaluation. Runs every case in three modes and writes metrics:
//   off       — intelligence disabled (Phase 4 baseline: context-free, faithful)
//   nocontext — ACI on, but context reset right before the target turn (same rules, no carry-over)
//   full      — ACI on with session context
// Two controllers (device A = English speaker, device B = Spanish speaker) see the
// dialogue as the real session would: own turns → outbound, other's → inbound.
//
//   npx vitest run --config vitest.eval.config.ts eval/intelligence
//
// The "translator" here is a deliberately HOSTILE STAND-IN, not a translation:
// it passes text through, except that it mistranslates person names that are
// also common words (Rose→rosa, Bill→factura …). It exists only to measure
// whether names survive translation. All other metrics are decision-level.

import { it, afterAll } from 'vitest';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { IntelligenceController } from '../../src/intelligence';
import type { InboundDecision, OutboundDecision } from '../../src/intelligence';

interface Case {
  id: string;
  category: string;
  turns: Array<{ speaker: 'A' | 'B'; lang: 'en' | 'es'; text: string }>;
  target: number;
  expect: {
    entities?: string[];
    senses?: Array<{ word: string; sense: string }>;
    references?: Array<{ word: string; antecedent: string | null }>;
    idioms?: string[];
    signals: string[];
    mustNotStore?: string[];
    noNotes?: boolean;
  };
}

const DIR = path.join(__dirname, 'cases');
// EVAL_SETS=dev runs only the developer set (used while tuning, before the freeze).
const WANT = (process.env.EVAL_SETS || 'dev,independent').split(',');
const SETS: Record<string, Case[]> = Object.fromEntries(
  WANT.map((n) => [n, JSON.parse(readFileSync(path.join(DIR, `${n}.json`), 'utf8')) as Case[]])
);
const MODES = ['off', 'nocontext', 'full'] as const;
type Mode = (typeof MODES)[number];
const THRESH = 0.5;

// ---- hostile stand-in translator (names that are common words get mistranslated)
const NAME_TRAPS: Record<string, string> = {
  Rose: 'rosa', Bill: 'factura', Grace: 'gracia', Hope: 'esperanza', Mark: 'marca', Will: 'voluntad', Rich: 'rico', Sunny: 'soleado',
  Lily: 'lirio', Daisy: 'margarita', Ruby: 'rubí', Faith: 'fe', Amber: 'ámbar', Iris: 'iris', Rob: 'robar', Art: 'arte', Summer: 'verano',
  Rosa: 'rose', Pilar: 'pillar', Mar: 'sea', Luz: 'light', Dolores: 'pains', Paloma: 'dove', Blanca: 'white', Rocío: 'dew', Esperanza: 'hope',
  Victoria: 'victory', Sol: 'sun', Luna: 'moon', Nieves: 'snows', Reyes: 'kings', Pastor: 'shepherd', Mayor: 'main', Calle: 'street', Ángeles: 'angels',
  Jack: 'gato', Penny: 'centavo', Dawn: 'amanecer', Joy: 'alegría', Pearl: 'perla', Robin: 'petirrojo', Frank: 'franco', Gene: 'gen',
};
const hostileMT = (text: string) => text.replace(/\p{L}+/gu, (w) => NAME_TRAPS[w] ?? w);

const norm = (s: string) => s.toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, '').replace(/[^\p{L}\p{N} ]/gu, ' ').replace(/\s+/g, ' ').trim();
const STOPG = new Set(['the', 'and', 'with', 'for', 'from', 'that', 'this', 'sense', 'meaning', 'kind', 'type']);
const gtoks = (g: string) => norm(g).split(' ').filter((t) => t.length >= 4 && !STOPG.has(t));
/** Gloss match: any content word of the expected gloss shares a 5-letter prefix with our gloss/id (documented method). */
const glossMatch = (expected: string, ours: string) => {
  const o = gtoks(ours);
  return gtoks(expected).some((e) => o.some((x) => x.slice(0, 5) === e.slice(0, 5)));
};

type Pred = { out?: OutboundDecision; inn?: InboundDecision; translated: string; ms: number[] };

function runCase(c: Case, mode: Mode): { pred: Pred; ctl: { A: IntelligenceController; B: IntelligenceController } } {
  const mk = () => new IntelligenceController(() => 0, { prefs: { enabled: mode !== 'off' } });
  const ctl = { A: mk(), B: mk() };
  ctl.A.setLangs('en-US', 'es-ES');
  ctl.B.setLangs('es-ES', 'en-US');
  const pred: Pred = { translated: '', ms: [] };
  c.turns.forEach((t, i) => {
    if (i === c.target && mode === 'nocontext') {
      ctl.A.reset();
      ctl.B.reset();
    }
    const speaker = t.speaker === 'A' ? ctl.A : ctl.B;
    const listener = t.speaker === 'A' ? ctl.B : ctl.A;
    const from = t.lang === 'en' ? 'en-US' : 'es-ES';
    const to = t.lang === 'en' ? 'es-ES' : 'en-US';
    const id = `${c.id}-${i}`;
    let t0 = performance.now();
    const d = speaker.outbound({ segmentId: id, text: t.text, lang: from, targetLang: to });
    pred.ms.push(performance.now() - t0);
    const raw = hostileMT(d.mtInput);
    const restored = speaker.restoreTranslation(d, raw);
    const translated = restored.ok ? restored.text : hostileMT(t.text);
    t0 = performance.now();
    const inn = listener.inbound({ segmentId: id, original: t.text, sourceLang: from, translated, listenerLang: to, assist: d.assist });
    pred.ms.push(performance.now() - t0);
    if (i === c.target) Object.assign(pred, { out: d, inn, translated });
  });
  return { pred, ctl };
}

// ---- metric accumulators
function empty() {
  return {
    cases: 0,
    senses: { total: 0, correct: 0, flaggedSafe: 0, overconfident: 0, missed: 0, expectAmbiguous: 0, ambiguousSurfaced: 0 },
    references: { total: 0, correct: 0, unresolvedSafe: 0, wrongGuess: 0, missed: 0, expectNull: 0, nullRespected: 0 },
    entities: { total: 0, preserved: 0 },
    idioms: { total: 0, found: 0, falsePositives: 0 },
    signals: {} as Record<string, { tp: number; fp: number; fn: number }>,
    calibration: {} as Record<string, { n: number; correct: number }>,
    neutral: { cases: 0, withNotes: 0 },
    sensitive: { strings: 0, leaked: 0 },
    meaning: { targets: 0, mtInputChanged: 0, addedWords: 0, originalAltered: 0 },
    msAll: [] as number[],
    failures: [] as string[],
  };
}
type Acc = ReturnType<typeof empty>;
const results: Record<string, Record<Mode, Acc>> = {};

function score(c: Case, mode: Mode, acc: Acc) {
  const { pred, ctl } = runCase(c, mode);
  acc.cases++;
  acc.msAll.push(...pred.ms);
  const f = pred.inn?.findings;
  const target = c.turns[c.target];
  const fail = (what: string) => mode === 'full' && acc.failures.push(`${c.id}: ${what}`);

  for (const e of c.expect.senses ?? []) {
    acc.senses.total++;
    const p = f?.senses.find((s) => s.word.toLowerCase() === e.word.toLowerCase());
    if (e.sense === 'ambiguous') {
      acc.senses.expectAmbiguous++;
      if (!p) (acc.senses.missed++, fail(`sense ${e.word}: not detected`));
      else if (p.sense === null) (acc.senses.correct++, acc.senses.ambiguousSurfaced++);
      else (acc.senses.overconfident++, fail(`sense ${e.word}: guessed ${p.sense}, expected ambiguous`));
    } else {
      if (!p) (acc.senses.missed++, fail(`sense ${e.word}: not detected`));
      else if (p.sense === null) (acc.senses.flaggedSafe++, fail(`sense ${e.word}: left ambiguous, expected ${e.sense}`));
      else if (glossMatch(e.sense, `${p.gloss} ${p.sense}`)) acc.senses.correct++;
      else (acc.senses.overconfident++, fail(`sense ${e.word}: chose ${p.sense}, expected ${e.sense}`));
    }
  }
  for (const e of c.expect.references ?? []) {
    acc.references.total++;
    const p = f?.references.find((r) => r.word.toLowerCase() === e.word.toLowerCase());
    if (e.antecedent === null) {
      acc.references.expectNull++;
      if (!p || p.antecedent === null) (acc.references.correct++, acc.references.nullRespected++);
      else (acc.references.wrongGuess++, fail(`ref ${e.word}: guessed ${p.antecedent}, expected none`));
    } else if (!p) (acc.references.missed++, fail(`ref ${e.word}: not detected`));
    else if (p.antecedent === null) (acc.references.unresolvedSafe++, fail(`ref ${e.word}: unresolved, expected ${e.antecedent}`));
    else if (norm(p.antecedent).includes(norm(e.antecedent)) || norm(e.antecedent).includes(norm(p.antecedent))) acc.references.correct++;
    else (acc.references.wrongGuess++, fail(`ref ${e.word}: ${p.antecedent} ≠ ${e.antecedent}`));
  }
  for (const e of c.expect.entities ?? []) {
    acc.entities.total++;
    if (pred.translated.includes(e)) acc.entities.preserved++;
    else fail(`entity ${e} not preserved`);
  }
  const predIdioms = f?.idioms ?? [];
  for (const e of c.expect.idioms ?? []) {
    acc.idioms.total++;
    if (predIdioms.some((p) => norm(e).includes(norm(p)) || norm(p).includes(norm(e)))) acc.idioms.found++;
    else fail(`idiom "${e}" missed`);
  }
  acc.idioms.falsePositives += predIdioms.filter((p) => !(c.expect.idioms ?? []).some((e) => norm(e).includes(norm(p)) || norm(p).includes(norm(e)))).length;

  const predSig = (pred.inn?.signals ?? []).filter((s) => s.confidence >= THRESH);
  const want = new Set(c.expect.signals);
  const labels = new Set([...want, ...predSig.map((s) => s.category)]);
  for (const l of labels) {
    if (l === 'topic-continuation' || l === 'unresolved-reference') continue; // not in the shared taxonomy
    const s = (acc.signals[l] ??= { tp: 0, fp: 0, fn: 0 });
    const p = predSig.some((x) => x.category === l);
    if (p && want.has(l)) s.tp++;
    else if (p) (s.fp++, fail(`signal ${l}: false positive`));
    else (s.fn++, fail(`signal ${l}: missed`));
  }
  for (const s of predSig) {
    if (s.category === 'topic-continuation' || s.category === 'unresolved-reference') continue;
    const b = s.confidence >= 0.85 ? '0.85-1.0' : s.confidence >= 0.7 ? '0.70-0.84' : '0.50-0.69';
    const k = (acc.calibration[b] ??= { n: 0, correct: 0 });
    k.n++;
    if (want.has(s.category)) k.correct++;
  }
  if (c.expect.noNotes) {
    acc.neutral.cases++;
    const n = (pred.inn?.notes ?? []).filter((x) => x.kind !== 'concise' && x.kind !== 'assisted');
    if (n.length) (acc.neutral.withNotes++, fail(`neutral: notes ${n.map((x) => x.kind).join(',')}`));
  }
  for (const s of c.expect.mustNotStore ?? []) {
    acc.sensitive.strings++;
    const dump = JSON.stringify([ctl.A.summary(), ctl.B.summary(), (ctl.A as never as { ctx: unknown }).ctx, (ctl.B as never as { ctx: unknown }).ctx]);
    if (dump.includes(s)) (acc.sensitive.leaked++, fail(`sensitive stored: ${s}`));
  }
  if (pred.out) {
    acc.meaning.targets++;
    const d = pred.out;
    if (d.mtInput !== target.text) acc.meaning.mtInputChanged++;
    // Words in MT input that are neither from the original, a placeholder, nor a lexicon edit.
    const allowed = new Set([...norm(target.text).split(' '), ...d.edits.flatMap((e) => norm(e.replacement).split(' '))]);
    acc.meaning.addedWords += norm(d.mtInput)
      .split(' ')
      .filter((w) => w && !/^qx\d+q$/.test(w) && !allowed.has(w)).length;
  }
}

for (const [set, cases] of Object.entries(SETS)) {
  results[set] = {} as Record<Mode, Acc>;
  for (const mode of MODES) {
    it(`${set} / ${mode}`, () => {
      const acc = empty();
      for (const c of cases) score(c, mode, acc);
      results[set][mode] = acc;
    });
  }
}

const stress: Record<string, unknown> = {};
it('long session: 600 turns, budgets hold, per-decision time', () => {
  const ctl = new IntelligenceController(() => 0);
  const lines = [
    'My friend Rose said the bank near Plaza Mayor opens at 9 am, right?',
    '¿Nos vemos allí con Lucía a las cinco?',
    'Sorry, not 5 pm, 6 pm. Can you call a taxi?',
    'Perdón, quise decir el jueves. Mi número es 612 345 678',
    'By the way, the match was great, our team won',
  ];
  const ms: number[] = [];
  for (let i = 0; i < 600; i++) {
    const t = lines[i % lines.length].replace('Rose', `Rose${i % 97}`);
    const es = i % 2 === 1;
    const t0 = performance.now();
    if (es) ctl.inbound({ segmentId: `s${i}`, original: t, sourceLang: 'es-ES', translated: t, listenerLang: 'en-US' });
    else ctl.outbound({ segmentId: `s${i}`, text: t, lang: 'en-US', targetLang: 'es-ES' });
    ms.push(performance.now() - t0);
  }
  const sorted = [...ms].sort((a, b) => a - b);
  Object.assign(stress, {
    turns: 600,
    p50ms: +sorted[300].toFixed(3),
    p95ms: +sorted[570].toFixed(3),
    maxms: +sorted.at(-1)!.toFixed(3),
    observedMax: ctl.maxObserved,
    budget: { turns: ctl.limits.maxTurns, chars: ctl.limits.maxTurnChars, entities: ctl.limits.maxEntities },
    storedPhone: JSON.stringify((ctl as never as { ctx: unknown }).ctx).includes('612 345 678'),
  });
});

afterAll(() => {
  const pct = (a: number[], p: number) => {
    const s = [...a].sort((x, y) => x - y);
    return s.length ? +s[Math.min(s.length - 1, Math.floor((p / 100) * s.length))].toFixed(3) : null;
  };
  const summary: Record<string, unknown> = {};
  for (const [set, modes] of Object.entries(results)) {
    summary[set] = Object.fromEntries(
      Object.entries(modes).map(([mode, a]) => {
        const sig = Object.fromEntries(
          Object.entries(a.signals)
            .sort()
            .map(([l, v]) => [l, { ...v, precision: v.tp + v.fp ? +(v.tp / (v.tp + v.fp)).toFixed(2) : null, recall: v.tp + v.fn ? +(v.tp / (v.tp + v.fn)).toFixed(2) : null }])
        );
        const micro = Object.values(a.signals).reduce((s, v) => ({ tp: s.tp + v.tp, fp: s.fp + v.fp, fn: s.fn + v.fn }), { tp: 0, fp: 0, fn: 0 });
        return [
          mode,
          {
            ...a,
            msAll: undefined,
            ms: { n: a.msAll.length, p50: pct(a.msAll, 50), p95: pct(a.msAll, 95), max: pct(a.msAll, 100) },
            signals: sig,
            signalsMicro: { ...micro, precision: micro.tp + micro.fp ? +(micro.tp / (micro.tp + micro.fp)).toFixed(2) : null, recall: micro.tp + micro.fn ? +(micro.tp / (micro.tp + micro.fn)).toFixed(2) : null },
          },
        ];
      })
    );
  }
  const out = path.join(__dirname, 'results');
  mkdirSync(out, { recursive: true });
  writeFileSync(path.join(out, `${process.env.EVAL_LABEL || 'phase5'}.json`), JSON.stringify({ generatedAt: new Date().toISOString(), node: process.version, threshold: THRESH, summary, stress }, null, 1));
});
