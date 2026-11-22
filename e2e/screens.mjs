// Phase 7: capture REAL rendered screens of the app in key states and viewports.
// Two Chromium pages, real signaling server, providers stubbed, scripted speech.
// Usage: node e2e/screens.mjs <outDir> [viewports=phone,tablet,desktop] [scale=1]
import path from 'node:path';
import { mkdirSync } from 'node:fs';
import { loadChromium, startStack, initScript, root } from './harness.mjs';

const out = path.resolve(process.argv[2] || path.join(root, 'docs/phase7/screens/current'));
const want = (process.argv[3] || 'phone,tablet,desktop').split(',');
const scale = Number(process.argv[4] || 1);
mkdirSync(out, { recursive: true });
const VIEWPORTS = { phone: { width: 390, height: 844 }, tablet: { width: 834, height: 1112 }, desktop: { width: 1440, height: 900 } };

const stack = await startStack({ apiPort: 8961, webPort: 4261, outDir: path.join(root, 'e2e/.dist-screens'), fetchOpts: { dict: { 'en|es': { 'where is the train station': '¿Dónde está la estación de tren?' }, 'es|en': { 'nos vemos a las cinco': "See you at five o'clock" } } } });
const chromium = await loadChromium();
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });
const shot = async (page, vp, name) => page.screenshot({ path: path.join(out, `${vp}-${name}.png`), fullPage: false });
const safe = async (fn) => {
  try {
    await fn();
  } catch (e) {
    console.log('  (skipped)', String(e.message).split('\n')[0]);
  }
};

try {
  for (const vp of want) {
    const mk = async (locale) => {
      const context = await browser.newContext({ locale, permissions: ['microphone'], viewport: VIEWPORTS[vp], deviceScaleFactor: scale, hasTouch: vp !== 'desktop', isMobile: vp === 'phone' });
      await context.addInitScript(initScript, {});
      const page = await context.newPage();
      return { context, page };
    };
    const host = await mk('en-US');
    const guest = await mk('es-ES');
    await host.page.goto(stack.web);
    await host.page.waitForTimeout(400);
    await shot(host.page, vp, '01-home');
    await host.page.getByLabel('Your name').fill('Ana');
    await safe(async () => {
      await host.page.getByRole('button', { name: /start a channel/i }).click();
      await host.page.waitForFunction(() => document.querySelectorAll('span.tnum').length === 6 && !document.body.innerText.includes('······'), null, { timeout: 8000 });
    });
    await host.page.waitForTimeout(300);
    await shot(host.page, vp, '02-start-waiting');
    const code = (await host.page.locator('span.tnum').allTextContents()).join('');
    await guest.page.goto(`${stack.web}/?join=${code}`);
    await guest.page.waitForTimeout(300);
    await shot(guest.page, vp, '03-join');
    await guest.page.getByLabel('Your name').fill('Luis');
    await safe(() => guest.page.getByRole('button', { name: /join a channel/i }).click());
    await safe(() => guest.page.getByRole('button', { name: /^join$/i }).click());
    await host.page.getByRole('button', { name: 'Push to talk' }).waitFor({ timeout: 15000 });
    await guest.page.getByRole('button', { name: 'Push to talk' }).waitFor({ timeout: 15000 });
    await host.page.waitForTimeout(600);
    await shot(host.page, vp, '04-consent');
    for (const p of [host.page, guest.page]) await safe(() => p.getByRole('button', { name: /allow microphone/i }).click());
    await host.page.waitForTimeout(400);
    await shot(host.page, vp, '05-call-idle');
    // holding + live caption
    await host.page.getByRole('button', { name: 'Push to talk' }).hover();
    await host.page.mouse.down();
    await host.page.waitForFunction(() => window.__srRunning());
    await host.page.evaluate(() => window.__say('where is the train', false));
    await host.page.waitForTimeout(300);
    await shot(host.page, vp, '06-holding');
    await host.page.evaluate(() => window.__say('where is the train station', true));
    await host.page.mouse.up();
    await guest.page.waitForTimeout(1500);
    await shot(guest.page, vp, '07-received');
    await guest.page.getByRole('button', { name: 'Push to talk' }).hover();
    await guest.page.mouse.down();
    await guest.page.waitForFunction(() => window.__srRunning());
    await guest.page.evaluate(() => window.__say('nos vemos a las cinco', true));
    await guest.page.mouse.up();
    await host.page.waitForTimeout(1500);
    await shot(host.page, vp, '08-after-reply');
    await safe(async () => {
      await host.page.getByRole('button', { name: /transcript/i }).first().click();
      await host.page.waitForTimeout(500);
      await shot(host.page, vp, '09-transcript');
      await host.page.keyboard.press('Escape');
      await safe(() => host.page.getByRole('button', { name: /close transcript/i }).click({ timeout: 1500 }));
    });
    await host.page.waitForTimeout(300);
    await safe(async () => {
      await host.page.getByRole('button', { name: /privacy|settings/i }).first().click();
      await host.page.waitForTimeout(500);
      await shot(host.page, vp, '10-privacy-or-settings');
      await safe(() => host.page.getByRole('button', { name: /close/i }).first().click({ timeout: 1500 }));
    });
    await host.page.waitForTimeout(300);
    await safe(async () => {
      await host.page.getByRole('button', { name: /conversation assistance|assist/i }).first().click({ timeout: 1500 });
      await host.page.waitForTimeout(500);
      await shot(host.page, vp, '11-assistance');
      await safe(() => host.page.getByRole('button', { name: /close/i }).first().click({ timeout: 1500 }));
    });
    await host.page.waitForTimeout(300);
    await safe(async () => {
      await host.page.getByRole('button', { name: /change language|language|english/i }).first().click({ timeout: 1500 });
      await host.page.waitForTimeout(500);
      await shot(host.page, vp, '12-language');
      await host.page.keyboard.press('Escape');
    });
    // connection lost on the guest side
    await safe(async () => {
      await guest.page.evaluate(() => window.__dropSockets());
      await guest.page.waitForTimeout(1200);
      await shot(guest.page, vp, '13-reconnecting');
    });
    await host.context.close();
    await guest.context.close();
    console.log(`captured ${vp}`);
  }
} finally {
  await browser.close();
  await stack.stop();
}
