// Voice wave: renders it in AIR driven by SCRIPTED recognizer events (simulated
// recognizer, real Chromium canvas), in dark and light, and checks that it is
// flat when nothing is reported. Screenshots → docs/voice_wave/.
import path from 'node:path';
import assert from 'node:assert/strict';
import { loadChromium, startStack, initScript, root } from './harness.mjs';

const stack = await startStack({ apiPort: 8991, webPort: 4291, outDir: path.join(root, 'e2e/.dist-wave'), fetchOpts: { dict: { 'hu|en': { 'Jó napot kívánok mindenkinek itt': 'Good afternoon to everyone here' } } } });
const chromium = await loadChromium();
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream'] });
const outDir = path.join(root, 'docs/voice_wave');
let code = 0;
try {
  for (const scheme of ['dark', 'light']) {
    const ctx = await browser.newContext({ colorScheme: scheme, permissions: ['microphone'], viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
    await ctx.addInitScript(initScript, {});
    const page = await ctx.newPage();
    const errors = [];
    page.on('pageerror', (e) => errors.push(e.message));
    await page.goto(`${stack.web}/#/air`);
    await page.getByRole('button', { name: 'Start listening' }).click();
    const d = page.getByRole('dialog', { name: 'Listening to the room' });
    if (await d.isVisible().catch(() => false)) await d.getByRole('button', { name: 'Start listening' }).click();
    await page.waitForFunction(() => window.__srRunning());
    const wave = page.getByTestId('voice-wave');
    await page.waitForTimeout(600);
    // Silence: nothing reported → the canvas holds only the idle line.
    const ink = () => wave.locator('canvas').evaluate((c) => {
      const g = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
      const rows = new Set();
      for (let i = 3; i < g.length; i += 4) if (g[i] > 40) rows.add(Math.floor((i - 3) / 4 / c.width));
      return rows.size; // vertical extent of drawn pixels
    });
    const idleRows = await ink();
    await wave.screenshot({ path: path.join(outDir, `air-idle-${scheme}.png`) });
    // Scripted speech: interim words arriving.
    const words = 'Jó napot kívánok mindenkinek itt'.split(' ');
    for (let i = 1; i <= words.length; i++) {
      await page.evaluate((t) => window.__say(t, false), words.slice(0, i).join(' '));
      await page.waitForTimeout(110);
    }
    const activeRows = await ink();
    await wave.screenshot({ path: path.join(outDir, `air-speech-${scheme}.png`) });
    await page.screenshot({ path: path.join(outDir, `air-screen-${scheme}.png`) });
    await page.evaluate((t) => window.__say(t, true), words.join(' '));
    await page.waitForTimeout(3500);
    const settledRows = await ink();
    console.log(`${scheme}: idle rows ${idleRows}, speaking rows ${activeRows}, after silence ${settledRows}`);
    assert.ok(idleRows <= 30, 'idle is one thin (tilted) line');
    assert.ok(activeRows > idleRows * 3, 'wave rises with reported speech');
    assert.ok(settledRows <= idleRows + 2, 'wave settles back to the idle line after silence');
    assert.deepEqual(errors, []);
    await ctx.close();
  }
  console.log('voice wave checks passed');
} catch (e) {
  console.error(e);
  code = 1;
} finally {
  await browser.close();
  await stack.stop?.();
  process.exit(code);
}
