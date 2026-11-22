// Phase 6: on-device phrasebook quality on an INDEPENDENT, blind-authored set
// (eval/local/cases/independent-phrases.json — written by a separate agent that
// never saw the phrasebook; run only after the phrasebook was frozen at 9e9b5e0).
//   npx vitest run --config vitest.eval.config.ts eval/local
import { it, afterAll } from 'vitest';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { Phrasebook } from '../../src/local/phrasebook';

interface Item { id: string; lang: 'en' | 'es'; text: string; inScope: boolean; references: string[]; meaningCritical: string[] }
const items: Item[] = JSON.parse(readFileSync(path.join(__dirname, 'cases/independent-phrases.json'), 'utf8'));
const fold = (s: string) => s.normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase().replace(/[^a-z0-9ñ ]/g, ' ').replace(/\s+/g, ' ').trim();
const out: Record<string, unknown> = {};

it('independent phrase set', () => {
  const pb = new Phrasebook();
  const t0 = performance.now();
  pb.load(() => performance.now());
  const loadMs = performance.now() - t0;
  const rows = items.map((it) => {
    const s = performance.now();
    const r = pb.translate(it.text, it.lang, it.lang === 'en' ? 'es' : 'en');
    const ms = performance.now() - s;
    const text = r?.text ?? null;
    const exact = text !== null && it.references.some((ref) => fold(ref) === fold(text));
    const critical = text !== null && it.meaningCritical.every((w) => fold(text).includes(fold(w)));
    return { id: it.id, lang: it.lang, inScope: it.inScope, covered: text !== null, exact, critical, coverage: r?.coverage ?? null, ms, input: it.text, output: text, refs: it.references };
  });
  const by = (f: (r: (typeof rows)[number]) => boolean) => rows.filter(f).length;
  const lat = rows.map((r) => r.ms).sort((a, b) => a - b);
  out.summary = {
    items: rows.length,
    inScope: by((r) => r.inScope),
    outOfScope: by((r) => !r.inScope),
    coveredInScope: by((r) => r.inScope && r.covered),
    coveredOutOfScope: by((r) => !r.inScope && r.covered),
    exactAmongCovered: by((r) => r.covered && r.exact),
    criticalPreservedAmongCovered: by((r) => r.covered && r.critical),
    coveredNotExact: by((r) => r.covered && !r.exact),
    coveredCriticalMissing: by((r) => r.covered && !r.critical),
    byLang: { en: { n: by((r) => r.lang === 'en'), covered: by((r) => r.lang === 'en' && r.covered) }, es: { n: by((r) => r.lang === 'es'), covered: by((r) => r.lang === 'es' && r.covered) } },
    loadMs: +loadMs.toFixed(3),
    translateMs: { p50: +lat[Math.floor(lat.length / 2)].toFixed(4), p95: +lat[Math.floor(lat.length * 0.95)].toFixed(4), max: +lat.at(-1)!.toFixed(4) },
  };
  out.coveredNotExact = rows.filter((r) => r.covered && !r.exact).map(({ id, input, output, refs, critical }) => ({ id, input, output, refs, critical }));
  out.coveredOutOfScope = rows.filter((r) => !r.inScope && r.covered).map(({ id, input, output }) => ({ id, input, output }));
  out.rows = rows.map(({ id, lang, inScope, covered, exact, critical, coverage }) => ({ id, lang, inScope, covered, exact, critical, coverage }));
});

afterAll(() => {
  mkdirSync(path.join(__dirname, 'results'), { recursive: true });
  writeFileSync(path.join(__dirname, 'results', `${process.env.EVAL_LABEL || 'phrasebook-frozen'}.json`), JSON.stringify({ generatedAt: new Date().toISOString(), node: process.version, ...out }, null, 1));
});
