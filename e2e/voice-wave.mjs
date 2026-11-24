// Logo wave (the Talktual mark in liquid glass) ↔ living waveform: renders Home
// and AIR in real Chromium driven by SCRIPTED recognizer events (simulated
// recognizer) in dark and light, and checks that
//  - Home's decorative mark and the background field render without errors,
//  - while listening in silence the mark stays the logo (morph ≈ 0),
//  - reported speech turns it into the waveform (morph → 1),
//  - after silence it returns to the logo,
//  - no page errors, console errors, or Three.js / WebGL warnings.
// Frames of the transformation → docs/voice_wave/.
import path from 'node:path';
import assert from 'node:assert/strict';
import { mkdirSync } from 'node:fs';
import { loadChromium, startStack, initScript, root } from './harness.mjs';

const stack = await startStack({ apiPort: 8991, webPort: 4291, outDir: path.join(root, 'e2e/.dist-wave'), fetchOpts: { dict: { 'hu|en': { 'Jó napot kívánok mindenkinek itt': 'Good afternoon to everyone here' } } } });
const chromium = await loadChromium();
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream'] });
const outDir = path.join(root, 'docs/voice_wave');
mkdirSync(outDir, { recursive: true });
let code = 0;
try {
  for (const scheme of ['dark', 'light']) {
    const ctx = await browser.newContext({ colorScheme: scheme, permissions: ['microphone'], viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
    await ctx.addInitScript(initScript, {});
    const page = await ctx.newPage();
    const problems = [];
    page.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`));
    page.on('console', (m) => {
      const t = m.text();
      // Screenshots read the canvas back, which the software GL driver reports as a stall; that is the test, not the app.
      if (/GPU stall due to ReadPixels/.test(t)) return;
      // The test server has no TTS key: /api/tts answers 503 by design and the browser voice speaks instead.
      if (/Failed to load resource/.test(t) && /\/api\/tts/.test(m.location()?.url ?? '')) return;
      if (m.type() === 'error' || (m.type() === 'warning' && /THREE|WebGL|GL_|shader/i.test(t))) problems.push(`${m.type()}: ${t}`);
    });
    // Home first: the decorative mark and the molten field must run cleanly.
    await page.goto(`${stack.web}/#/`);
    await page.waitForFunction(() => document.querySelector('[data-webgl="on"]'), null, { timeout: 15000 });
    await page.waitForTimeout(1500);
    await page.goto(`${stack.web}/#/air`);
    await page.getByRole('button', { name: 'Start listening' }).click();
    const d = page.getByRole('dialog', { name: 'Listening to the room' });
    if (await d.isVisible().catch(() => false)) await d.getByRole('button', { name: 'Start listening' }).click();
    await page.waitForFunction(() => window.__srRunning());
    const stage = page.getByTestId('voice-wave');
    const orb = stage.locator('[data-morph]');
    await page.waitForFunction(() => document.querySelector('[data-testid="voice-wave"] [data-webgl="on"]'), null, { timeout: 15000 });
    const morph = async () => Number(await orb.getAttribute('data-morph'));
    await page.waitForTimeout(1200);
    const idle = await morph();
    await stage.screenshot({ path: path.join(outDir, `logo-idle-${scheme}.jpg`), type: 'jpeg', quality: 86 });

    // Scripted speech: interim words arriving steadily.
    const words = 'Jó napot kívánok mindenkinek itt és most is'.split(' ');
    let peak = 0;
    let shot = 0;
    for (let round = 0; round < 3; round++) {
      for (let i = 1; i <= words.length; i++) {
        await page.evaluate((t) => window.__say(t, false), words.slice(0, i).join(' ') + (round ? ` ${round}` : ''));
        await page.waitForTimeout(90);
        const m = await morph();
        peak = Math.max(peak, m);
        if (scheme === 'dark' && shot < 4 && m > 0.12 * (shot + 1) + 0.05) {
          await stage.screenshot({ path: path.join(outDir, `logo-morph-${++shot}-${scheme}.jpg`), type: 'jpeg', quality: 86 });
        }
      }
    }
    await stage.screenshot({ path: path.join(outDir, `logo-wave-${scheme}.jpg`), type: 'jpeg', quality: 86 });
    await page.screenshot({ path: path.join(outDir, `air-screen-${scheme}.jpg`), type: 'jpeg', quality: 86 });
    await page.evaluate((t) => window.__say(t, true), words.join(' '));
    await page.waitForFunction(() => Number(document.querySelector('[data-testid="voice-wave"] [data-morph]')?.getAttribute('data-morph')) < 0.05, null, { timeout: 8000 });
    const settled = await morph();
    console.log(`${scheme}: morph idle ${idle.toFixed(2)}, peak while speaking ${peak.toFixed(2)}, after silence ${settled.toFixed(2)}`);
    assert.ok(idle <= 0.05, 'silent: stays the logo');
    assert.ok(peak >= 0.6, 'reported speech: becomes the waveform');
    assert.ok(settled <= 0.05, 'after silence: back to the logo');
    assert.deepEqual(problems, []);
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
