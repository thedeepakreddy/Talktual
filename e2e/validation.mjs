// Phase 8.5 — validation (test) mode in real Chromium: opt-in only, exports
// content-free timings, and is absent by default. SIMULATED recognizer.
import assert from 'node:assert/strict';
import path from 'node:path';
import { loadChromium, startStack, initScript, root } from './harness.mjs';

const results = [];
async function step(name, fn) {
  try {
    await fn();
    results.push({ name, ok: true });
    console.log(`  ✔ ${name}`);
  } catch (err) {
    results.push({ name, ok: false });
    console.log(`  ✖ ${name}\n      ${String(err?.message || err).split('\n').join('\n      ')}`);
  }
}
const PHRASES = { 'Jó estét kívánok': 'Good evening', 'Hol a kijárat': 'Where is the exit' };
const stack = await startStack({ apiPort: 8985, webPort: 4285, outDir: path.join(root, 'e2e/.dist-ambient'), fetchOpts: { dict: { 'hu|en': PHRASES } } });
const chromium = await loadChromium();
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });
try {
  console.log(`Chromium ${browser.version()} — validation mode (simulated recognizer)`);
  const open = async (query) => {
    const ctx = await browser.newContext({ locale: 'en-US', permissions: ['microphone'], viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
    await ctx.addInitScript(initScript, {});
    await ctx.addInitScript(() => localStorage.setItem('talktual.ambient.consent.v2.browser-speech', 'granted'));
    const page = await ctx.newPage();
    await page.goto(`${stack.web}/${query}#/air`);
    return { ctx, page };
  };

  await step('default: no test mode and no validation hook', async () => {
    const { ctx, page } = await open('');
    assert.equal(await page.getByLabel('Test mode').count(), 0);
    assert.equal(await page.evaluate(() => typeof window.__talktualValidation), 'undefined');
    await ctx.close();
  });

  await step('?validate=1: mark end of speech, export a summary with stage timings and no content', async () => {
    const { ctx, page } = await open('?validate=1');
    await page.getByLabel('Test mode').waitFor();
    await page.getByRole('button', { name: 'Start listening' }).click();
    await page.waitForFunction(() => window.__srRunning());
    for (const hu of Object.keys(PHRASES)) {
      await page.waitForFunction(() => window.__srRunning(), null, { timeout: 5000 });
      await page.evaluate((t) => window.__say(t.split(' ')[0], false), hu);
      await page.getByRole('button', { name: /Mark end of speech/ }).click();
      await page.evaluate((t) => window.__say(t, true), hu);
      await page.waitForFunction((e) => window.__spoken.some((s) => s.text === e), PHRASES[hu], { timeout: 8000 });
      await page.waitForTimeout(300);
    }
    const sum = await page.evaluate(() => window.__talktualValidation.summary());
    assert.equal(sum.schema, 'talktual.ambient.validation.v1');
    assert.equal(sum.env.browser, 'chrome');
    assert.equal(sum.measured.translate_ms.n, 2);
    assert.equal(sum.humanMarked.mark_to_audio_start_ms.n, 2);
    const json = JSON.stringify(sum);
    for (const [hu, en] of Object.entries(PHRASES)) {
      for (const w of [...hu.split(' '), ...en.split(' ')].filter((x) => x.length > 3)) assert.ok(!json.includes(w), `content leaked: ${w}`);
    }
    // Download path works (no exception, message shown).
    const dl = page.waitForEvent('download');
    await page.getByRole('button', { name: 'Export' }).click();
    const file = await dl;
    assert.match(file.suggestedFilename(), /^talktual-validation-.*\.json$/);
    await page.getByLabel('Test mode').getByText('Exported').waitFor();
    await page.getByRole('button', { name: 'Clear', exact: true }).click();
    assert.equal(await page.evaluate(() => window.__talktualValidation.summary().events.length), 0);
    // Nothing persisted.
    const stored = await page.evaluate(() => JSON.stringify({ ...localStorage }));
    assert.ok(!/validation|u1|translate_ms/.test(stored), stored);
    await ctx.close();
  });
} finally {
  await browser.close();
  await stack.stop();
}
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed}/${results.length} validation-mode steps passed`);
process.exit(failed ? 1 : 0);
