// Phase 6 E2E: processing modes in two real Chromium pages against the real
// signaling server (provider fetch stubbed — no live calls). Every network
// request each page makes is recorded, so "On-device only sends nothing to
// recognition/translation/voice endpoints" is checked on the wire, not inferred.
// Speech recognition/synthesis are scripted fakes; on-device voices for the
// guest are SIMULATED (the test hardware has none).
//
// Usage: node e2e/processing.mjs

import assert from 'node:assert/strict';
import path from 'node:path';
import { loadChromium, startStack, pairPeers, root } from './harness.mjs';

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
const INFERENCE = /\/api\/(translate|tts|stt)\b/;
function recordRequests(page) {
  const log = [];
  page.on('request', (r) => log.push(r.url()));
  return { inference: () => log.filter((u) => INFERENCE.test(u)), all: () => [...log] };
}
async function typeMsg(page, text) {
  const box = page.getByRole('textbox', { name: 'Type a message' });
  if (!(await box.count())) await page.getByRole('button', { name: 'Type' }).click();
  await box.fill(text);
  await page.getByRole('button', { name: 'Send' }).click();
}
async function setMode(page, label) {
  await page.getByRole('button', { name: 'Privacy and processing' }).click();
  await page.getByRole('radio', { name: label }).check();
  await page.getByRole('button', { name: 'Close privacy' }).click();
}
async function transcript(page) {
  await page.getByRole('button', { name: 'Show transcript' }).click();
  const t = await page.getByRole('dialog', { name: 'Transcript' }).innerText();
  await page.getByRole('button', { name: 'Close transcript' }).click();
  return t;
}
const spoken = (page) => page.evaluate(() => window.__spoken.map((s) => s.text));

const chromium = await loadChromium();
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });
let exitCode = 0;
const stack = await startStack({ apiPort: 8941, webPort: 4241, outDir: path.join(root, 'e2e/.dist-proc') });
let host, guest, hostNet, guestNet;
try {
  console.log(`Chromium ${browser.version()} — processing modes (default build)`);
  await step('pair (guest has SIMULATED on-device Spanish/English voices); consent', async () => {
    ({ host, guest } = await pairPeers(browser, stack.web, { guest: { simulatedLocalVoices: ['es-ES', 'en-US'] } }));
    hostNet = recordRequests(host.page);
    guestNet = recordRequests(guest.page);
    for (const p of [host.page, guest.page]) await p.getByRole('button', { name: /allow microphone/i }).click();
  });

  await step('On-device only: pressing to talk starts NO recognizer and sends no audio; typing is offered', async () => {
    await setMode(host.page, 'On-device only');
    const before = await host.page.evaluate(() => window.__sr.length);
    await host.page.getByRole('button', { name: 'Push to talk' }).hover();
    await host.page.mouse.down();
    await host.page.waitForTimeout(300);
    await host.page.mouse.up();
    assert.equal(await host.page.evaluate(() => window.__sr.length), before, 'a speech recognizer was created');
    await host.page.getByText(/type your message instead/i).waitFor({ timeout: 3000 });
    await host.page.getByRole('textbox', { name: 'Type a message' }).waitFor();
  });

  await step('On-device only: covered phrase translated on the device; uncovered sent as-is; ZERO inference requests from either page', async () => {
    const h0 = hostNet.inference().length;
    const g0 = guestNet.inference().length;
    const p0 = stack.providerCalls.length;
    await typeMsg(host.page, 'where is the train station');
    await guest.page.getByText('¿Dónde está la estación de tren?').first().waitFor({ timeout: 8000 });
    await typeMsg(host.page, 'the new exhibition about medieval maps opens next week');
    await guest.page.waitForTimeout(1500);
    assert.equal(hostNet.inference().length - h0, 0, `host inference calls: ${hostNet.inference().slice(h0)}`);
    assert.equal(guestNet.inference().length - g0, 0, `guest inference calls: ${guestNet.inference().slice(g0)}`);
    assert.equal(stack.providerCalls.length - p0, 0);
    const gt = await transcript(guest.page);
    assert.match(gt, /They asked for on-device processing/);
    assert.match(gt, /medieval maps/); // shown as sent, untranslated
    // Guest spoke the covered phrase with its (simulated) on-device voice only.
    const s = await spoken(guest.page);
    assert.ok(s.includes('¿Dónde está la estación de tren?'), JSON.stringify(s));
    assert.match(await transcript(host.page), /Typed · Translated on this device/);
  });

  await step('Hybrid: covered phrase stays on the device, uncovered phrase uses the cloud (one request)', async () => {
    await setMode(host.page, 'Hybrid');
    const h0 = hostNet.inference().length;
    await typeMsg(host.page, 'thank you very much');
    await guest.page.getByText('Muchas gracias').first().waitFor({ timeout: 8000 });
    assert.equal(hostNet.inference().length - h0, 0);
    await typeMsg(host.page, 'the conference starts after lunch');
    await guest.page.getByText(/conference starts after lunch/).first().waitFor({ timeout: 8000 });
    assert.equal(hostNet.inference().filter((u) => /translate/.test(u)).length - h0, 1);
  });

  await step('Offline (browser network emulation): status shown; only on-device; recovery without replay or duplicates', async () => {
    await guest.context.setOffline(true);
    await guest.page.getByText(/Offline — on-device only/).waitFor({ timeout: 5000 });
    const g0 = guestNet.inference().length;
    await typeMsg(guest.page, 'buenas noches');
    await guest.page.waitForTimeout(500);
    assert.equal(guestNet.inference().length - g0, 0, 'request attempted while offline');
    await guest.context.setOffline(false);
    await guest.page.getByText(/Offline — on-device only/).waitFor({ state: 'detached', timeout: 10000 });
    await host.page.getByText('Good evening').first().waitFor({ timeout: 15000 });
    await host.page.waitForTimeout(1500);
    const n = (await transcript(host.page)).split('Good evening').length - 1;
    assert.equal(n, 1, `delivered ${n} times`);
  });

  await step('no page errors; VoiceMirror off', async () => {
    for (const p of [host, guest]) assert.deepEqual(p.errors, []);
    assert.equal(await host.page.getByRole('button', { name: /voice match/i }).count(), 0);
  });
} catch {
  exitCode = 1;
} finally {
  await host?.context.close().catch(() => {});
  await guest?.context.close().catch(() => {});
  await stack.stop();
}

const off = await startStack({ apiPort: 8942, webPort: 4242, outDir: path.join(root, 'e2e/.dist-proc-off'), buildEnv: { VITE_PROCESSING_MODES: '0' } });
try {
  console.log('Rollback build (VITE_PROCESSING_MODES=0)');
  await step('rollback: no privacy controls or typed input; speech goes through the Phase 5 path', async () => {
    const { host: h, guest: g } = await pairPeers(browser, off.web);
    for (const p of [h.page, g.page]) await p.getByRole('button', { name: /allow microphone/i }).click();
    assert.equal(await h.page.getByRole('button', { name: 'Privacy and processing' }).count(), 0);
    assert.equal(await h.page.getByRole('button', { name: 'Type' }).count(), 0);
    await h.page.getByRole('button', { name: 'Push to talk' }).hover();
    await h.page.mouse.down();
    await h.page.waitForFunction(() => window.__srRunning());
    await h.page.evaluate(() => window.__say('where is the train station'));
    await h.page.mouse.up();
    await g.page.waitForFunction(() => window.__spoken.length > 0, null, { timeout: 8000 });
    assert.ok(off.providerCalls.some((c) => c.q === 'where is the train station'), 'expected the cloud translator (Phase 5 behaviour)');
    for (const p of [h, g]) assert.deepEqual(p.errors, []);
    await h.context.close();
    await g.context.close();
  });
} catch {
  exitCode = 1;
} finally {
  await off.stop();
  await browser.close();
}
console.log(`\n${results.filter((r) => r.ok).length}/${results.length} processing checks passed`);
process.exit(exitCode);
