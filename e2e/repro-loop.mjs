// Communication-loop reproduction (reliability task). Runs the SAME scenarios
// against any source tree (e.g. a worktree of origin/main = the code that would
// be deployed) and the current branch, in two real Chromium pages with the real
// signaling server (provider fetch stubbed — no live calls).
//
// The speech recognizer is a scripted fake. Unlike the other suites, it can
// behave like a real cloud recognizer: interim text while talking, and the
// FINAL result arriving some time AFTER stop() — or never (onend without a
// final, as Chrome on Android often does). Outcomes are sanitized booleans.
//
// Usage: node e2e/repro-loop.mjs [treePath] [label]

import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { writeFileSync, mkdirSync } from 'node:fs';
import { loadChromium, startStack, pairPeers, root, makeFakeFetch } from './harness.mjs';

const tree = path.resolve(process.argv[2] || root);
// REPRO_ASSERT=1: exit non-zero unless every scenario delivered text and audio both ways (regression mode).
const label = process.argv[3] || 'current';

/** Patch the fake recognizer: stop() delivers the final after `finalDelayMs` (or never), then ends. */
function realisticRecognizer() {
  const S = window.webkitSpeechRecognition;
  if (!S) return;
  const origStop = S.prototype.stop;
  S.prototype.stop = function () {
    const cfg = window.__srBehavior;
    if (!cfg) return origStop.call(this);
    if (!this.running) return;
    this.running = false;
    window.__srLog.push({ ev: 'stop', t: Date.now() });
    const self = this;
    setTimeout(() => {
      if (cfg.finalDelayMs !== null && self.onresult && window.__pendingInterim) {
        const res = [{ transcript: window.__pendingInterim, confidence: 0.9 }];
        res.isFinal = true;
        self.onresult({ resultIndex: 0, results: [res] });
      }
      setTimeout(() => self.onend && self.onend(), 20);
    }, cfg.finalDelayMs ?? cfg.endWithoutFinalMs ?? 200);
  };
}

async function talkRealistic(page, text, behavior) {
  const ptt = page.getByRole('button', { name: 'Push to talk' });
  await ptt.hover();
  await page.mouse.down();
  await page.waitForFunction(() => window.__srRunning());
  await page.evaluate(
    async ([t, b]) => {
      window.__srBehavior = b;
      const words = t.split(' ');
      for (let i = 1; i <= words.length; i++) {
        window.__pendingInterim = words.slice(0, i).join(' ');
        window.__say(window.__pendingInterim, false); // interim only, like a real engine mid-phrase
        await new Promise((r) => setTimeout(r, 60));
      }
      if (b.finalBeforeRelease) window.__say(t, true);
    },
    [text, behavior]
  );
  await page.mouse.up();
}

const SCENARIOS = [
  { id: 'final-before-release', behavior: { finalBeforeRelease: true, finalDelayMs: 0 } },
  { id: 'final-300ms-after-stop', behavior: { finalDelayMs: 300 } },
  { id: 'final-900ms-after-stop', behavior: { finalDelayMs: 900 } },
  { id: 'final-1600ms-after-stop', behavior: { finalDelayMs: 1600 } },
  { id: 'no-final-onend-only', behavior: { finalDelayMs: null, endWithoutFinalMs: 250 } },
];
const PHRASES = { A: 'where is the train station', B: 'donde esta la estacion de tren' };

// REPRO_SERVER_DIR: run THAT server version as a separate process (with fetch
// stubbed via REPRO_FETCH_STUB, so nothing leaves the machine) behind this tree's
// frontend — to test frontend/backend version skew.
const only = process.env.REPRO_SCENARIOS ? process.env.REPRO_SCENARIOS.split(',') : null;
const scenarios = only ? SCENARIOS.filter((s) => only.includes(s.id)) : SCENARIOS;
let stack;
if (process.env.REPRO_SERVER_DIR) {
  const srv = spawn(process.execPath, ['-r', process.env.REPRO_FETCH_STUB, 'server.js'], { cwd: path.join(process.env.REPRO_SERVER_DIR, 'signaling-server'), env: { ...process.env, PORT: '8951' }, stdio: 'ignore' });
  await new Promise((r) => setTimeout(r, 1500));
  const dist = path.join(tree, 'e2e/.dist-repro');
  spawnSync('npx', ['vite', 'build', '--outDir', dist, '--emptyOutDir', '--logLevel', 'error'], { cwd: tree, env: { ...process.env, VITE_SIGNALING_URL: 'ws://127.0.0.1:8951' }, stdio: 'inherit' });
  try { await fetch('http://127.0.0.1:4251'); throw new Error('port 4251 already serving'); } catch (e) { if (String(e?.message).includes('already')) throw e; }
  const preview = spawn(path.join(tree, 'node_modules/.bin/vite'), ['preview', '--outDir', dist, '--port', '4251', '--strictPort', '--host', '127.0.0.1'], { cwd: tree, stdio: 'ignore' });
  for (let i = 0; i < 100; i++) { try { if ((await fetch('http://127.0.0.1:4251')).ok) break; } catch {} await new Promise((r) => setTimeout(r, 200)); }
  stack = { web: 'http://127.0.0.1:4251', providerCalls: [], stop: async () => { preview.kill(); srv.kill(); } };
} else {
  // REPRO_SERVER_TTS=1: the server's REAL /api/tts path with a STUBBED ElevenLabs
  // that streams synthetic PCM (a 440 Hz tone) — the listener's browser then
  // plays it through its real Web Audio (no speech synthesis involved).
  const serverOverrides = {};
  if (process.env.REPRO_SERVER_TTS) {
    const base = makeFakeFetch({});
    serverOverrides.elevenLabsApiKey = 'stub-not-a-real-key';
    serverOverrides.fetch = async (url, init) => {
      const u = new URL(String(url));
      if (u.hostname === 'api.elevenlabs.io' && u.pathname.includes('/text-to-speech/')) {
        const rate = Number((u.searchParams.get('output_format') || 'pcm_24000').split('_')[1]) || 24000;
        const samples = Math.round(rate * 0.6);
        const buf = Buffer.alloc(samples * 2);
        for (let i = 0; i < samples; i++) buf.writeInt16LE(Math.round(Math.sin((2 * Math.PI * 440 * i) / rate) * 8000), i * 2);
        return new Response(buf, { status: 200, headers: { 'content-type': 'audio/pcm' } });
      }
      return base(url, init);
    };
  }
  stack = await startStack({ apiPort: 8951, webPort: 4251, cwd: tree, outDir: path.join(tree, 'e2e/.dist-repro'), serverOverrides });
}
const chromium = await loadChromium();
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });
const rows = [];
try {
  for (const sc of scenarios) {
    const { host, guest } = await pairPeers(browser, stack.web);
    for (const p of [host, guest]) {
      await p.page.evaluate(realisticRecognizer);
      await p.page.evaluate(() => {
        // Observation only: count real AudioBufferSourceNode starts (Web Audio playback).
        window.__webAudioStarts = 0;
        const orig = AudioBufferSourceNode.prototype.start;
        AudioBufferSourceNode.prototype.start = function (...a) {
          if (this.buffer && this.buffer.length > 1) window.__webAudioStarts++;
          return orig.apply(this, a);
        };
      });
      const allow = p.page.getByRole('button', { name: /allow microphone/i });
      if (await allow.count()) await allow.click();
    }
    for (const [dir, speaker, listener, phrase] of [
      ['A→B (en→es)', host, guest, PHRASES.A],
      ['B→A (es→en)', guest, host, PHRASES.B],
    ]) {
      const calls0 = stack.providerCalls.filter((c) => c.kind === 'translate').length;
      const heard0 = await listener.page.evaluate(() => window.__spoken.length);
      const wa0 = await listener.page.evaluate(() => window.__webAudioStarts);
      await talkRealistic(speaker.page, phrase, sc.behavior);
      await listener.page.waitForTimeout(3500);
      const translateCalls = process.env.REPRO_SERVER_DIR ? null : stack.providerCalls.filter((c) => c.kind === 'translate').length - calls0;
      const peerHeard = (await listener.page.evaluate(() => window.__spoken.length)) - heard0;
      const peerText = await listener.page.evaluate((p) => document.body.innerText.includes(p), phrase.split(' ').slice(-2).join(' '));
      const webAudio = (await listener.page.evaluate(() => window.__webAudioStarts)) - wa0;
      rows.push({ tree: label, scenario: sc.id, direction: dir, translateCalls, peerTextShown: peerText, peerAudioUtterances: peerHeard, peerWebAudioChunks: webAudio, errors: speaker.errors.length + listener.errors.length });
    }
    await host.context.close();
    await guest.context.close();
  }
} finally {
  await browser.close();
  await stack.stop();
}
console.table(rows);
if (process.env.REPRO_ASSERT) {
  const bad = rows.filter((r) => !r.peerTextShown || (r.peerAudioUtterances + (r.peerWebAudioChunks ?? 0)) < 1 || r.errors);
  if (bad.length) {
    console.error(`${bad.length} scenario(s) failed`);
    process.exitCode = 1;
  } else console.log(`${rows.length}/${rows.length} loop checks passed`);
}
mkdirSync(path.join(root, 'docs/reliability'), { recursive: true });
writeFileSync(path.join(root, `docs/reliability/repro-${label}.json`), JSON.stringify({ at: new Date().toISOString(), tree, label, rows }, null, 1));
