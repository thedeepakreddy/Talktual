// End-to-end: Talktual with the Communication Engine recognizer.
//
// Real pieces: two Chromium pages, the production build with
// VITE_ASR_ENGINE=ce, the signaling server (in-process; translation and TTS
// providers stubbed), the Talktual ↔ CE bridge with the real sherpa-onnx
// model, and Chromium's fake microphone playing a SYNTHETIC TTS fixture
// (communication-engine/fixtures/speech; no human speech).
//
// Checks: consent shows the CE route; holding push-to-talk streams the mic
// through the AudioWorklet to CE; CE's transcript reaches the translator and
// the other peer; the call's processing is labelled CE.
//
// Prerequisites: cargo build --release in communication-engine/adapters/talktual-bridge,
// CE_SHERPA_MODEL_DIR set (adapters/ce-asr-sherpa/scripts/fetch-model.sh).
// Usage: CE_SHERPA_MODEL_DIR=… node e2e/ce-bridge.mjs

import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { loadChromium, startStack, pairPeers, root } from './harness.mjs';

const BRIDGE = path.join(root, 'communication-engine/adapters/talktual-bridge/target/release/talktual-bridge');
const FIXTURE = path.join(root, 'communication-engine/fixtures/speech/dev_kal16_station.wav');
const API_PORT = 8931;
const WEB_PORT = 4231;
const BRIDGE_PORT = 8191;

if (!process.env.CE_SHERPA_MODEL_DIR) {
  console.log('CE_SHERPA_MODEL_DIR is not set — this test needs the real model; not run.');
  process.exit(2);
}
if (!fs.existsSync(BRIDGE)) {
  console.log(`bridge binary missing: ${BRIDGE} (cargo build --release there first)`);
  process.exit(2);
}

/** 0.5 s silence + fixture + 1.5 s silence, 16 kHz mono PCM16 (Chromium loops it). */
function micFile() {
  const wav = fs.readFileSync(FIXTURE);
  const dataAt = wav.indexOf('data') + 8;
  const pcm = wav.subarray(dataAt);
  const pad = (ms) => Buffer.alloc(16 * ms * 2);
  const body = Buffer.concat([pad(500), pcm, pad(1500)]);
  const hdr = Buffer.alloc(44);
  hdr.write('RIFF', 0);
  hdr.writeUInt32LE(36 + body.length, 4);
  hdr.write('WAVEfmt ', 8);
  hdr.writeUInt32LE(16, 16);
  hdr.writeUInt16LE(1, 20);
  hdr.writeUInt16LE(1, 22);
  hdr.writeUInt32LE(16000, 24);
  hdr.writeUInt32LE(32000, 28);
  hdr.writeUInt16LE(2, 32);
  hdr.writeUInt16LE(16, 34);
  hdr.write('data', 36);
  hdr.writeUInt32LE(body.length, 40);
  const out = path.join(root, 'e2e/.ce-mic.wav');
  fs.writeFileSync(out, Buffer.concat([hdr, body]));
  return out;
}

const results = [];
async function step(name, fn) {
  const t0 = Date.now();
  try {
    await fn();
    results.push({ name, ok: true });
    console.log(`  ✔ ${name} (${Date.now() - t0}ms)`);
  } catch (err) {
    results.push({ name, ok: false });
    console.log(`  ✖ ${name}\n      ${String(err?.message || err).split('\n').join('\n      ')}`);
    throw err;
  }
}

const FINALS = path.join(root, 'e2e/.ce-finals.jsonl');
fs.rmSync(FINALS, { force: true });
const bridgeLog = [];
const bridge = spawn(BRIDGE, ['--listen', `127.0.0.1:${BRIDGE_PORT}`, '--allow-origin', `http://127.0.0.1:${WEB_PORT}`, '--log-finals', FINALS], {
  env: process.env,
  stdio: ['ignore', 'pipe', 'pipe'],
});
bridge.stdout.on('data', (d) => bridgeLog.push(String(d)));
bridge.stderr.on('data', (d) => bridgeLog.push(String(d)));
const t0 = Date.now();
while (!bridgeLog.join('').includes('listening')) {
  if (Date.now() - t0 > 60000 || bridge.exitCode !== null) {
    console.log(bridgeLog.join(''));
    throw new Error('bridge did not start');
  }
  await new Promise((r) => setTimeout(r, 100));
}

const chromium = await loadChromium();
const browser = await chromium.launch({
  args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream', `--use-file-for-fake-audio-capture=${micFile()}`, '--autoplay-policy=no-user-gesture-required'],
});
let exitCode = 0;
const stack = await startStack({
  apiPort: API_PORT,
  webPort: WEB_PORT,
  outDir: path.join(root, 'e2e/.dist-ce'),
  buildEnv: { VITE_ASR_ENGINE: 'ce', VITE_CE_BRIDGE_URL: `ws://127.0.0.1:${BRIDGE_PORT}` },
});
let host, guest;
try {
  console.log(`Chromium ${browser.version()} — Talktual with the CE recognizer (real model, synthetic mic)`);
  console.log(`  ${bridgeLog.join('').trim().split('\n')[0]}`);
  await step('pair; the consent sheet discloses the CE route', async () => {
    ({ host, guest } = await pairPeers(browser, stack.web));
    await host.page.getByText(/Communication Engine bridge running on this computer/).waitFor({ timeout: 10000 });
    for (const p of [host.page, guest.page]) await p.getByRole('button', { name: /allow microphone/i }).click();
  });

  await step('push-to-talk streams the mic through CE; the transcript reaches the translator and the other peer', async () => {
    await host.page.getByRole('button', { name: 'Push to talk' }).hover();
    await host.page.mouse.down();
    await host.page.waitForTimeout(12000); // > two loops of the fixture
    await host.page.mouse.up();
    // Wiring, not accuracy: the model sometimes hears "drain station" on
    // Chromium's processed fake mic (accuracy is measured by the evaluation).
    await guest.page.waitForFunction(() => window.__spoken.some((s) => /could you tell me where the \w+ station/i.test(s.text)), null, { timeout: 20000 });
    const qs = stack.providerCalls.filter((c) => c.kind === 'translate').map((c) => c.q);
    const heard = await guest.page.evaluate(() => window.__spoken.map((s) => s.text));
    console.log(`      translator received: ${JSON.stringify(qs)}`);
    console.log(`      guest heard: ${JSON.stringify(heard)}`);
    assert.ok(qs.some((q) => /could you tell me where the \w+ station/i.test(q)), 'CE transcript translated');
    // The browser's own recognizer was never used.
    const srStarts = await host.page.evaluate(() => window.__srLog.filter((e) => e.ev === 'start').length);
    assert.equal(srStarts, 0, 'Web Speech must not run in the CE build');
  });

  await step('the opt-in transcript log records finals with latencies (human-test scoring path)', async () => {
    const lines = fs.readFileSync(FINALS, 'utf8').trim().split('\n').map((l) => JSON.parse(l));
    assert.ok(lines.some((l) => /WHERE THE \w+ STATION/.test(l.text)), JSON.stringify(lines));
    assert.ok(lines.every((l) => typeof l.audioMs === 'number' && l.adapter === 'ce.asr.sherpa-zipformer'));
    console.log(`      log: ${lines.map((l) => `${JSON.stringify(l.text)} firstPartial=${l.firstPartialAfterUtteranceStartMs}ms afterRelease=${l.finalAfterReleaseMs ?? '–'}ms`).join('; ')}`);
  });

  await step('Engine Lab: all four phases live from the real engine (hold to talk)', async () => {
    const lab = await host.context.newPage();
    const labErrors = [];
    lab.on('pageerror', (e) => labErrors.push(e.message));
    await lab.setViewportSize({ width: 1100, height: 900 });
    await lab.goto(`${stack.web}/#/engine`);
    await lab.getByRole('heading', { name: 'Engine Lab' }).waitFor();
    await lab.getByLabel('Bridge address').fill(`ws://127.0.0.1:${BRIDGE_PORT}`);
    await lab.getByRole('button', { name: 'Connect' }).click();
    await lab.getByText(/Connected · 1 recognizer/).waitFor({ timeout: 10000 });
    const hold = lab.getByRole('button', { name: 'Hold to talk' });
    await hold.hover();
    await lab.mouse.down();
    await lab.waitForTimeout(7000);
    await lab.mouse.up();
    const finals = lab.getByRole('list', { name: 'Final utterances' });
    await finals.waitFor({ timeout: 20000 });
    await lab.waitForFunction(() => /station/i.test(document.querySelector('[aria-label="Final utterances"]')?.textContent ?? ''), null, { timeout: 20000 });
    const text = await lab.locator('main').innerText();
    assert.match(text, /ce\.asr\.sherpa-zipformer/, 'Phase 4 recognizer shown');
    assert.match(text, /ce\.vad\.spectral/, 'Phase 3 detector shown');
    assert.match(text, /16000 Hz · 1 ch · i16|48000 Hz · 1 ch · i16|44100 Hz · 1 ch · i16/, 'Phase 2 negotiated format shown');
    assert.match(text, /healthy|running/, 'Phase 1 health shown');
    const events = await lab.getByRole('list', { name: 'Events' }).locator('li').count();
    assert.ok(events > 10, `event log has ${events} entries`);
    console.log(`      lab finals: ${JSON.stringify(await finals.innerText())}`);
    await lab.screenshot({ path: path.join(root, 'e2e/.ce-lab.png'), fullPage: true });
    assert.deepEqual(labErrors, []);
    await lab.close();
  });

  await step('no page errors', async () => {
    assert.deepEqual(host.errors, []);
    assert.deepEqual(guest.errors, []);
  });
} catch {
  exitCode = 1;
} finally {
  await browser.close();
  await stack.stop();
  bridge.kill('SIGINT');
}
console.log(`\n${results.filter((r) => r.ok).length}/${results.length} passed`);
process.exit(exitCode);
