// Phase 6 REAL measurements on this machine (no simulation): Node (with GC
// control) and Chromium headless, same bundle. Writes docs/phase6/measurements.json.
//   node eval/local/measure.mjs
import { execFileSync, spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync, mkdirSync, statSync } from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadChromium } from '../../e2e/harness.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '../..');
const out = path.join(here, '.bundle');
mkdirSync(out, { recursive: true });
execFileSync(path.join(root, 'node_modules/.bin/esbuild'), [path.join(here, 'measure-entry.ts'), '--bundle', '--format=esm', `--outfile=${out}/local.mjs`, '--log-level=error']);
execFileSync(path.join(root, 'node_modules/.bin/esbuild'), [path.join(here, 'measure-entry.ts'), '--bundle', '--format=esm', '--minify', `--outfile=${out}/local.min.mjs`, '--log-level=error']);
const phrases = JSON.parse(readFileSync(path.join(here, 'cases/independent-phrases.json'), 'utf8')).map((p) => [p.text, p.lang]);

// ---- Node (child process so --expose-gc applies) ----
const nodeScript = `
import { Phrasebook, ProcessingPolicy } from ${JSON.stringify(out + '/local.mjs')};
const phrases = ${JSON.stringify(phrases)};
const pct = (a, p) => { const s = [...a].sort((x, y) => x - y); return +s[Math.min(s.length - 1, Math.floor(p / 100 * s.length))].toFixed(4); };
gc(); gc();
const h0 = process.memoryUsage().heapUsed;
const pb = new Phrasebook();
const t0 = performance.now(); pb.load(() => performance.now()); const coldLoadMs = performance.now() - t0;
gc(); gc();
const h1 = process.memoryUsage().heapUsed;
const first = performance.now(); pb.translate('where is the train station', 'en', 'es'); const firstMs = performance.now() - first;
const ms = [];
const c0 = process.cpuUsage(); const w0 = performance.now();
for (let r = 0; r < 50; r++) for (const [t, l] of phrases) { const s = performance.now(); pb.translate(t, l, l === 'en' ? 'es' : 'en'); ms.push(performance.now() - s); }
const wall = performance.now() - w0; const cpu = process.cpuUsage(c0);
pb.unload(); gc(); gc();
const h2 = process.memoryUsage().heapUsed;
const t1 = performance.now(); pb.load(() => performance.now()); const warmReloadMs = performance.now() - t1;
const pol = new ProcessingPolicy('hybrid', () => 0, 10);
const p0 = performance.now(); for (let i = 0; i < 100000; i++) pol.plan('translate', { deviceAvailable: i % 2 === 0, cloudAvailable: true }); const planUs = (performance.now() - p0) * 1000 / 100000;
console.log(JSON.stringify({ coldLoadMs: +coldLoadMs.toFixed(3), firstTranslateMs: +firstMs.toFixed(4), warmReloadMs: +warmReloadMs.toFixed(3), heapLoadedKB: Math.round((h1 - h0) / 1024), heapAfterUnloadKB: Math.round((h2 - h0) / 1024), translate: { n: ms.length, p50: pct(ms, 50), p95: pct(ms, 95), max: pct(ms, 100) }, cpuMsPerCall: +((cpu.user + cpu.system) / 1000 / ms.length).toFixed(4), cpuUtilisation: +(((cpu.user + cpu.system) / 1000) / wall).toFixed(2), policyPlanMicroseconds: +planUs.toFixed(3) }));
`;
const nodeRes = spawnSync(process.execPath, ['--expose-gc', '--input-type=module', '-e', nodeScript], { encoding: 'utf8' });
if (nodeRes.status !== 0) throw new Error(nodeRes.stderr);
const node = JSON.parse(nodeRes.stdout.trim().split('\n').at(-1));

// ---- Chromium ----
const bundle = readFileSync(`${out}/local.mjs`, 'utf8');
const srv = http.createServer((req, res) => {
  if (req.url === '/local.mjs') { res.setHeader('content-type', 'text/javascript'); return res.end(bundle); }
  res.setHeader('content-type', 'text/html'); res.end('<p>m</p>');
}).listen(4297);
const chromium = await loadChromium();
const b = await chromium.launch();
const page = await b.newPage();
await page.goto('http://localhost:4297/');
const browserRes = await page.evaluate(async (phrases) => {
  const pct = (a, p) => { const s = [...a].sort((x, y) => x - y); return +s[Math.min(s.length - 1, Math.floor(p / 100 * s.length))].toFixed(4); };
  const i0 = performance.now(); const m = await import('/local.mjs'); const importMs = performance.now() - i0;
  const mem0 = performance.memory?.usedJSHeapSize ?? null;
  const pb = new m.Phrasebook(); const t0 = performance.now(); pb.load(() => performance.now()); const coldLoadMs = performance.now() - t0;
  const ms = [];
  for (let r = 0; r < 50; r++) for (const [t, l] of phrases) { const s = performance.now(); pb.translate(t, l, l === 'en' ? 'es' : 'en'); ms.push(performance.now() - s); }
  const mem1 = performance.memory?.usedJSHeapSize ?? null;
  return { importMs: +importMs.toFixed(2), coldLoadMs: +coldLoadMs.toFixed(3), translate: { n: ms.length, p50: pct(ms, 50), p95: pct(ms, 95), max: pct(ms, 100) }, heapDeltaKB_coarse: mem0 !== null ? Math.round((mem1 - mem0) / 1024) : null, timerResolutionNote: 'performance.now() is coarsened in browsers (≈0.1 ms without cross-origin isolation)' };
}, phrases);
await b.close(); srv.close();

// ---- bundle cost ----
const appJs = (() => { try { const d = path.join(root, 'dist/assets'); return execFileSync('bash', ['-c', `ls -l ${d}/*.js | awk '{s+=$5} END {print s}'`]).toString().trim(); } catch { return null; } })();
const result = {
  at: new Date().toISOString(),
  hardware: { cpu: os.cpus()[0].model, cores: os.cpus().length, memGB: Math.round(os.totalmem() / 2 ** 30), gpu: 'none', platform: `${process.platform} ${process.arch}`, node: process.version, chromium: b.version?.() ?? '157 (headless)' },
  battery: 'not measurable (container, no battery)', thermal: 'not measurable (container)',
  localModuleBytes: { unminified: statSync(`${out}/local.mjs`).size, minified: statSync(`${out}/local.min.mjs`).size },
  appBundleBytes: appJs ? Number(appJs) : null,
  node, chromium: browserRes,
};
mkdirSync(path.join(root, 'docs/phase6'), { recursive: true });
writeFileSync(path.join(root, 'docs/phase6/measurements.json'), JSON.stringify(result, null, 1));
console.log(JSON.stringify(result, null, 1));
