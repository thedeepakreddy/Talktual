// Server performance / stability benchmark. Spawns a server process (path given
// as argv[2], default: the current signaling-server/server.js) with no
// provider keys, then measures from a client process:
//
//   * startup time to first /health 200
//   * HTTP latency: /health, /api/capabilities, /api/translate (same-language
//     passthrough — no provider call, so this isolates server overhead)
//   * WebSocket relay round-trip latency between two peers in one session
//   * repeated join/leave cycles: server RSS before/after
//   * many concurrent sessions: RSS growth per connected pair
//
// No external network calls are made. Usage:
//   node e2e/perf.mjs [path/to/server.js] [port]

import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(path.join(root, 'signaling-server/package.json'));
const WebSocket = require('ws');

const serverPath = path.resolve(process.argv[2] || path.join(root, 'signaling-server/server.js'));
const PORT = Number(process.argv[3] || 8931);
const HTTP = `http://127.0.0.1:${PORT}`;
const WS = `ws://127.0.0.1:${PORT}`;

const pct = (arr, p) => {
  const s = [...arr].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.floor((p / 100) * s.length))];
};
const fmt = (arr) => `p50 ${pct(arr, 50).toFixed(2)}ms  p95 ${pct(arr, 95).toFixed(2)}ms  p99 ${pct(arr, 99).toFixed(2)}ms  (n=${arr.length})`;
const rssMb = (pid) => {
  const m = readFileSync(`/proc/${pid}/status`, 'utf8').match(/VmRSS:\s+(\d+)/);
  return m ? Number(m[1]) / 1024 : NaN;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const t0 = performance.now();
const child = spawn(process.execPath, [serverPath], {
  cwd: path.dirname(serverPath),
  env: { PATH: process.env.PATH, PORT: String(PORT) },
  stdio: ['ignore', 'ignore', 'inherit'],
});
for (;;) {
  try {
    if ((await fetch(`${HTTP}/health`)).ok) break;
  } catch {
    /* starting */
  }
  await sleep(5);
}
const startupMs = performance.now() - t0;

const out = { server: path.relative(root, serverPath), node: process.version, startupMs: +startupMs.toFixed(1) };
console.log(`server: ${out.server}  node ${process.version}`);
console.log(`startup to first /health 200: ${startupMs.toFixed(1)}ms   idle RSS ${rssMb(child.pid).toFixed(1)} MB`);

async function timeHttp(n, fn) {
  const lat = [];
  for (let i = 0; i < 50; i++) await fn(); // warm-up
  for (let i = 0; i < n; i++) {
    const s = performance.now();
    await fn();
    lat.push(performance.now() - s);
  }
  return lat;
}

const health = await timeHttp(2000, async () => (await fetch(`${HTTP}/health`)).arrayBuffer());
console.log(`GET /health              ${fmt(health)}`);
const caps = await timeHttp(2000, async () => (await fetch(`${HTTP}/api/capabilities`)).arrayBuffer());
console.log(`GET /api/capabilities    ${fmt(caps)}`);
const tr = await timeHttp(2000, async () =>
  (
    await fetch(`${HTTP}/api/translate`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ text: 'hello there', from: 'en-US', to: 'en-GB' }),
    })
  ).arrayBuffer()
);
console.log(`POST /api/translate (pt) ${fmt(tr)}`);

const open = () =>
  new Promise((res, rej) => {
    const w = new WebSocket(WS);
    w.on('open', () => res(w));
    w.on('error', rej);
  });
const nextMsg = (w, pred = () => true) =>
  new Promise((res) => {
    const h = (d) => {
      const m = JSON.parse(d.toString());
      if (pred(m)) {
        w.off('message', h);
        res(m);
      }
    };
    w.on('message', h);
  });
async function pair(code) {
  const a = await open();
  const b = await open();
  const ra = nextMsg(a, (m) => m.type === 'ready');
  const rb = nextMsg(b, (m) => m.type === 'ready');
  a.send(JSON.stringify({ type: 'join', sessionId: code }));
  await nextMsg(a, (m) => m.type === 'role');
  b.send(JSON.stringify({ type: 'join', sessionId: code }));
  await Promise.all([ra, rb]);
  return [a, b];
}
const closeAll = (ws) => Promise.all(ws.map((w) => new Promise((r) => (w.on('close', r), w.close()))));

// Relay round trip: A -> server -> B, B echoes -> server -> A.
{
  const [a, b] = await pair('PERF01');
  b.on('message', (d) => {
    const m = JSON.parse(d.toString());
    if (m.type === 'app') b.send(JSON.stringify({ type: 'app', payload: { ...m.payload, echo: true } }));
  });
  const lat = [];
  const payload = { t: 'partial', text: 'the quick brown fox jumps over the lazy dog', lang: 'en-US' };
  for (let i = 0; i < 3000; i++) {
    const s = performance.now();
    const got = nextMsg(a, (m) => m.type === 'app');
    a.send(JSON.stringify({ type: 'app', payload }));
    await got;
    if (i >= 100) lat.push(performance.now() - s);
  }
  console.log(`WS relay round trip      ${fmt(lat)}`);
  out.relayRtt = { p50: pct(lat, 50), p95: pct(lat, 95), p99: pct(lat, 99) };
  await closeAll([a, b]);
}

// Repeated join/leave.
{
  const before = rssMb(child.pid);
  const t = performance.now();
  for (let i = 0; i < 1000; i++) {
    const code = 'JL' + String(i).padStart(4, '0');
    const ws = await pair(code);
    await closeAll(ws);
  }
  await sleep(300);
  const after = rssMb(child.pid);
  const ms = performance.now() - t;
  console.log(`1000 join/leave cycles   ${ms.toFixed(0)}ms total (${(ms / 1000).toFixed(2)}ms/cycle)   RSS ${before.toFixed(1)} -> ${after.toFixed(1)} MB`);
  out.joinLeave = { totalMs: Math.round(ms), rssBefore: +before.toFixed(1), rssAfter: +after.toFixed(1) };
}

// Concurrent sessions.
{
  const before = rssMb(child.pid);
  const socks = [];
  for (let i = 0; i < 250; i++) socks.push(...(await pair('CC' + String(i).padStart(4, '0'))));
  await sleep(300);
  const during = rssMb(child.pid);
  await closeAll(socks);
  await sleep(500);
  const after = rssMb(child.pid);
  console.log(
    `250 concurrent pairs     RSS ${before.toFixed(1)} -> ${during.toFixed(1)} MB (≈${(((during - before) * 1024) / 250).toFixed(0)} KB/pair), after close ${after.toFixed(1)} MB`
  );
  out.concurrent = { rssBefore: +before.toFixed(1), rssDuring: +during.toFixed(1), rssAfter: +after.toFixed(1) };
}

const alive = await fetch(`${HTTP}/health`).then((r) => r.ok).catch(() => false);
console.log(`server alive at end: ${alive}`);
child.kill();
console.log('JSON ' + JSON.stringify(out));
