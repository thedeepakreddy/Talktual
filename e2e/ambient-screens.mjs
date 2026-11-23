// Phase 8: real renders of Live translate in key states (simulated recognizer).
// Usage: node e2e/ambient-screens.mjs <outDir> [phone,tablet,desktop] [scale] [dark|light]
import path from 'node:path';
import { mkdirSync } from 'node:fs';
import { loadChromium, startStack, initScript, root } from './harness.mjs';

const out = path.resolve(process.argv[2] || path.join(root, 'docs/phase8/screens'));
const want = (process.argv[3] || 'phone,tablet,desktop').split(',');
const scale = Number(process.argv[4] || 1);
const scheme = process.argv[5] === 'light' ? 'light' : 'dark';
mkdirSync(out, { recursive: true });
const VIEWPORTS = { phone: { width: 390, height: 844 }, tablet: { width: 834, height: 1112 }, desktop: { width: 1440, height: 900 } };
const dict = {
  'hu|en': {
    'Jó napot kívánok': 'Good afternoon',
    'Elnézést, hol van a pályaudvar': 'Excuse me, where is the railway station',
    'Egyenesen, aztán balra a második utcán': 'Straight on, then left at the second street',
    'Két kávét kérek, az egyiket tejjel': 'Two coffees please, one with milk',
    'Mennyibe kerül a jegy Bécsbe': 'How much is the ticket to Vienna',
  },
};
const stack = await startStack({ apiPort: 8983, webPort: 4283, outDir: path.join(root, 'e2e/.dist-ambient'), fetchOpts: { dict } });
const chromium = await loadChromium();
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });
const sfx = scheme === 'light' ? '-light' : '';
const shot = async (page, vp, name) => {
  await page.waitForTimeout(350);
  await page.screenshot({ path: path.join(out, `${vp}-${name}${sfx}.png`) });
};
const say = (page, t, fin = true) => page.evaluate(([x, f]) => window.__say(x, f), [t, fin]);

try {
  for (const vp of want) {
    const ctx = await browser.newContext({ locale: 'en-US', colorScheme: scheme, permissions: ['microphone'], viewport: VIEWPORTS[vp], deviceScaleFactor: scale, hasTouch: vp !== 'desktop', isMobile: vp === 'phone' });
    await ctx.addInitScript(initScript, {});
    const page = await ctx.newPage();
    await page.goto(stack.web);
    await shot(page, vp, '01-home-entry');
    await page.getByRole('button', { name: /translate what’s around you/i }).click();
    await shot(page, vp, '02-setup');
    await page.getByRole('button', { name: 'Start listening' }).click();
    await page.getByRole('dialog', { name: 'Listening to the room' }).waitFor();
    await shot(page, vp, '03-consent');
    await page.getByRole('dialog').getByRole('button', { name: 'Start listening' }).click();
    await page.waitForFunction(() => window.__srRunning());
    await shot(page, vp, '04-listening-empty');
    for (const t of Object.keys(dict['hu|en'])) {
      await page.waitForFunction(() => window.__srRunning(), null, { timeout: 5000 });
      await say(page, t);
      await page.waitForTimeout(500);
    }
    await page.waitForFunction(() => window.__srRunning(), null, { timeout: 5000 });
    await say(page, 'és utána megyünk a', false);
    await page.waitForTimeout(150);
    await shot(page, vp, '05-listening-feed');
    await say(page, 'és utána megyünk a piacra', true);
    await page.waitForTimeout(800);
    await page.getByRole('button', { name: 'Pause listening' }).click();
    await shot(page, vp, '06-paused');
    await page.getByRole('button', { name: 'Resume listening' }).click();
    await page.waitForFunction(() => window.__srRunning());
    for (const t of ['I think that the train is late again', 'and you know it is not the first time this week']) {
      await page.waitForFunction(() => window.__srRunning(), null, { timeout: 5000 });
      await say(page, t);
      await page.waitForTimeout(400);
    }
    await page.getByText(/This sounds like English/).waitFor();
    await shot(page, vp, '07-language-hint');
    await page.getByRole('button', { name: 'Dismiss' }).click();
    await page.getByRole('button', { name: 'Live translate settings' }).click();
    await shot(page, vp, '08-settings');
    await page.getByRole('dialog').evaluate((d) => d.querySelector('.scroll-area')?.scrollTo(0, 99999));
    await shot(page, vp, '08b-settings-scrolled');
    await page.getByRole('radio', { name: 'On-device' }).click();
    await page.keyboard.press('Escape');
    await page.waitForTimeout(300);
    await shot(page, vp, '09-on-device-unavailable');
    await ctx.close();

    // A browser without speech recognition.
    const ctx2 = await browser.newContext({ locale: 'en-US', colorScheme: scheme, permissions: ['microphone'], viewport: VIEWPORTS[vp], deviceScaleFactor: scale, hasTouch: vp !== 'desktop', isMobile: vp === 'phone' });
    await ctx2.addInitScript(initScript, { noWebSpeech: true });
    const p2 = await ctx2.newPage();
    await p2.goto(stack.web);
    await p2.evaluate(() => localStorage.setItem('talktual.ambient.consent.v1.browser-speech', 'granted'));
    await p2.getByRole('button', { name: /translate what’s around you/i }).click();
    await p2.getByRole('button', { name: 'Start listening' }).click();
    await shot(p2, vp, '10-no-speech-recognition');
    await ctx2.close();
    console.log(`captured ${vp} (${scheme})`);
  }
} finally {
  await browser.close();
  await stack.stop();
}
