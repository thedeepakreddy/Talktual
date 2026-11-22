// Emulated compatibility scenarios in Chromium. These are NOT real Firefox,
// Safari or mobile runs; they exercise the code paths those platforms take.
//
//   1. No Web Speech API (Firefox-like) + server STT configured: the guest
//      records with a real MediaRecorder (Chromium's fake microphone), the
//      server transcription provider is stubbed. Disclosure names ElevenLabs;
//      hands-free is not offered.
//   2. No Web Speech + no server STT: the user is told up front and the talk
//      button is disabled; listening still works.
//   3. Phone viewport + touch (Pixel 5 descriptor): no horizontal scroll,
//      controls reachable, consent sheet usable.
//
// Usage: node e2e/compat.mjs

import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { loadChromium, startStack, pairPeers, talk, initScript, openSettings, closeSettings, readCallCode } from './harness.mjs';

const require = createRequire(import.meta.url);
const results = [];
async function step(name, fn) {
  const t0 = Date.now();
  try {
    await fn();
    results.push({ name, ok: true });
    console.log(`  ✔ ${name} (${Date.now() - t0}ms)`);
  } catch (err) {
    results.push({ name, ok: false, err: String(err?.message || err) });
    console.log(`  ✖ ${name}\n      ${String(err?.message || err).split('\n').join('\n      ')}`);
  }
}

const chromium = await loadChromium();
const launch = () => chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });

// ---------------- 1. server-STT path ----------------
{
  const stack = await startStack({
    apiPort: 8914,
    webPort: 4214,
    // A placeholder key only switches the server's STT route on; every
    // ElevenLabs call goes to the stubbed fetch (STT returns text, TTS 500s).
    serverOverrides: { elevenLabsApiKey: 'e2e-placeholder-not-a-key' },
    fetchOpts: { sttText: 'hola desde firefox' },
  });
  const browser = await launch();
  try {
    let pair;
    await step('1: Firefox-like guest (no Web Speech) joins; disclosure names the server route', async () => {
      pair = await pairPeers(browser, stack.web, { guest: { noWebSpeech: true } });
      const dialog = pair.guest.page.getByRole('dialog', { name: /before you talk/i });
      await dialog.waitFor({ timeout: 5000 });
      assert.match(await dialog.textContent(), /ElevenLabs/);
      await dialog.getByRole('button', { name: /allow microphone/i }).click();
      await pair.host.page.getByRole('button', { name: /allow microphone/i }).click();
      await openSettings(pair.guest.page);
      assert.equal(await pair.guest.page.getByRole('radio', { name: 'Hands-free' }).count(), 0, 'hands-free not offered without streaming ASR');
      await closeSettings(pair.guest.page);
    });

    await step('1: guest holds, a real MediaRecorder captures the fake mic, transcript is translated and heard by host', async () => {
      const ptt = pair.guest.page.getByRole('button', { name: 'Push to talk' });
      await ptt.hover();
      await pair.guest.page.mouse.down();
      await pair.guest.page.waitForTimeout(1200); // record ~1s of the fake device
      await pair.guest.page.mouse.up();
      await pair.host.page.waitForFunction(() => window.__spoken.some((s) => s.text === '[es|en] hola desde firefox'), null, {
        timeout: 15000,
      });
      const sttCalls = stack.providerCalls.filter((c) => c.kind === 'stt').length;
      assert.equal(sttCalls, 1);
    });

    await step('1: no page errors', async () => assert.deepEqual([...pair.host.errors, ...pair.guest.errors], []));
  } finally {
    await browser.close();
    await stack.stop();
  }
}

// ---------------- 2. no STT anywhere ----------------
{
  const stack = await startStack({ apiPort: 8915, webPort: 4215 });
  const browser = await launch();
  try {
    await step('2: no Web Speech and no server STT → told up front, talk disabled, listening works', async () => {
      const pair = await pairPeers(browser, stack.web, { guest: { noWebSpeech: true } });
      await pair.guest.page.getByText(/speech recognition isn’t available/i).waitFor({ timeout: 5000 });
      assert.equal(await pair.guest.page.getByRole('button', { name: 'Push to talk' }).isDisabled(), true);
      assert.equal(await pair.guest.page.getByRole('dialog', { name: /before you talk/i }).count(), 0, 'no consent prompt when nothing can be captured');
      await pair.host.page.getByRole('button', { name: /allow microphone/i }).click();
      await talk(pair.host.page, 'can you hear me');
      await pair.guest.page.waitForFunction(() => window.__spoken.some((s) => s.text === '[en|es] can you hear me'));
    });
  } finally {
    await browser.close();
    await stack.stop();
  }
}

// ---------------- 3. phone viewport ----------------
{
  const stack = await startStack({ apiPort: 8916, webPort: 4216 });
  const browser = await launch();
  try {
    await step('3: Pixel 5 emulation (393px, touch) — home, consent and call screens fit without horizontal scroll', async () => {
      const { devices } = require('playwright');
      const ctx = await browser.newContext({ ...devices['Pixel 5'], locale: 'en-US', permissions: ['microphone'] });
      await ctx.addInitScript(initScript, {});
      const page = await ctx.newPage();
      const errors = [];
      page.on('pageerror', (e) => errors.push(e.message));
      const noHScroll = async (label) => {
        const { sw, cw } = await page.evaluate(() => ({ sw: document.documentElement.scrollWidth, cw: document.documentElement.clientWidth }));
        assert.ok(sw <= cw, `${label}: scrollWidth ${sw} > clientWidth ${cw}`);
      };
      await page.goto(stack.web);
      await noHScroll('home');
      await page.getByLabel('Your name').fill('Ana');
      await page.getByRole('button', { name: /^start a call$/i }).tap();
      const code = await readCallCode(page);
      await noHScroll('invite');
      // Second participant on desktop.
      const other = await browser.newContext({ locale: 'es-ES', permissions: ['microphone'] });
      await other.addInitScript(initScript, {});
      const op = await other.newPage();
      await op.goto(`${stack.web}/?join=${code}`);
      await op.getByLabel('Your name').fill('Luis');
      await op.getByRole('button', { name: /^join$/i }).click();
      const dialog = page.getByRole('dialog', { name: /before you talk/i });
      await dialog.waitFor({ timeout: 15000 });
      const allow = dialog.getByRole('button', { name: /allow microphone/i });
      await page.waitForTimeout(1000); // let the sheet's slide-in animation finish
      const box = await allow.boundingBox();
      console.log(`      Allow button box: y=${box?.y?.toFixed(0)} h=${box?.height?.toFixed(0)}; viewport h=${page.viewportSize().height}`);
      const vp = page.viewportSize();
      assert.ok(box && box.y + box.height <= vp.height, 'Allow button is on screen');
      await allow.tap();
      await noHScroll('call');
      const ptt = await page.getByRole('button', { name: 'Push to talk' }).boundingBox();
      assert.ok(ptt && ptt.width >= 44 && ptt.height >= 44, 'touch target ≥ 44px');
      assert.deepEqual(errors, []);
    });
  } finally {
    await browser.close();
    await stack.stop();
  }
}

const passed = results.filter((r) => r.ok).length;
console.log(`\n${passed}/${results.length} compatibility (emulated) steps passed`);
process.exit(passed === results.length ? 0 : 1);
