// Phase 7: capture REAL rendered screens of the app in key states and viewports.
// Two Chromium pages, real signaling server, providers stubbed, scripted speech.
// Usage: node e2e/screens.mjs <outDir> [viewports=phone,tablet,desktop] [scale=1] [scheme=dark|light]
import path from 'node:path';
import { mkdirSync } from 'node:fs';
import { loadChromium, startStack, initScript, root, readCallCode } from './harness.mjs';

const out = path.resolve(process.argv[2] || path.join(root, 'docs/phase7/screens/current'));
const want = (process.argv[3] || 'phone,tablet,desktop').split(',');
const scale = Number(process.argv[4] || 1);
const scheme = process.argv[5] === 'light' ? 'light' : 'dark';
mkdirSync(out, { recursive: true });
const VIEWPORTS = { phone: { width: 390, height: 844 }, tablet: { width: 834, height: 1112 }, desktop: { width: 1440, height: 900 } };

const dict = {
  'en|es': {
    'where is the train station': '¿Dónde está la estación de tren?',
    'is it far from here': '¿Está lejos de aquí?',
    'great thank you': 'Genial, gracias',
  },
  'es|en': {
    'está a diez minutos andando': "It's a ten-minute walk",
    'sigue recto y gira a la izquierda en la plaza': 'Go straight and turn left at the square',
  },
};
const stack = await startStack({ apiPort: 8961, webPort: 4261, outDir: path.join(root, 'e2e/.dist-screens'), fetchOpts: { dict } });
const chromium = await loadChromium();
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });
const suffix = scheme === 'light' ? '-light' : '';
const shot = async (page, vp, name) => {
  await page.waitForTimeout(350); // let sheet/transition motion settle
  await page.screenshot({ path: path.join(out, `${vp}-${name}${suffix}.png`), fullPage: false });
};
const safe = async (label, fn) => {
  try {
    await fn();
  } catch (e) {
    console.log(`  (skipped ${label})`, String(e.message).split('\n')[0]);
  }
};
async function utter(page, text) {
  await page.getByRole('button', { name: 'Push to talk' }).hover();
  await page.mouse.down();
  await page.waitForFunction(() => window.__srRunning());
  await page.evaluate((t) => window.__say(t, true), text);
  await page.mouse.up();
  await page.waitForTimeout(900);
}
const openSettings = (page) => page.getByRole('button', { name: 'Call settings' }).click();
const closeSheet = (page) => page.keyboard.press('Escape');

try {
  for (const vp of want) {
    const mk = async (locale) => {
      const context = await browser.newContext({
        locale,
        colorScheme: scheme,
        permissions: ['microphone'],
        viewport: VIEWPORTS[vp],
        deviceScaleFactor: scale,
        hasTouch: vp !== 'desktop',
        isMobile: vp === 'phone',
      });
      await context.addInitScript(initScript, {});
      const page = await context.newPage();
      return { context, page };
    };
    const host = await mk('en-US');
    const guest = await mk('es-ES');

    await host.page.goto(stack.web);
    await shot(host.page, vp, '01-home');
    await host.page.getByRole('button', { name: 'Start a call' }).click();
    await shot(host.page, vp, '01b-home-name-required');
    await host.page.getByLabel('Your name').fill('Ana');
    await host.page.getByRole('button', { name: 'Start a call' }).click();
    const code = await readCallCode(host.page);
    await shot(host.page, vp, '02-start-waiting');

    // Code entry path (separate page, not joined).
    await guest.page.goto(stack.web);
    await guest.page.getByLabel('Your name').fill('Luis');
    await guest.page.getByRole('button', { name: 'Join with a code' }).click();
    await guest.page.getByLabel('Call code').fill(code.slice(0, 3));
    await shot(guest.page, vp, '03b-join-code');

    // Invite-link path (the one actually used).
    await guest.page.goto(`${stack.web}/?join=${code}`);
    await shot(guest.page, vp, '03-join-invite');
    await guest.page.getByLabel('Your name').fill('Luis');
    await guest.page.getByRole('button', { name: /^join$/i }).click();
    await host.page.getByRole('button', { name: 'Push to talk' }).waitFor({ timeout: 15000 });
    await guest.page.getByRole('button', { name: 'Push to talk' }).waitFor({ timeout: 15000 });
    await host.page.getByRole('dialog', { name: /before you talk/i }).waitFor();
    await shot(host.page, vp, '04-consent');
    for (const p of [host.page, guest.page]) await p.getByRole('button', { name: /allow microphone/i }).click();
    await shot(host.page, vp, '05-call-idle');

    // Holding with a live caption.
    await host.page.getByRole('button', { name: 'Push to talk' }).hover();
    await host.page.mouse.down();
    await host.page.waitForFunction(() => window.__srRunning());
    await host.page.evaluate(() => window.__say('where is the train', false));
    await shot(host.page, vp, '06-holding');
    await host.page.evaluate(() => window.__say('where is the train station', true));
    await host.page.mouse.up();
    await guest.page.waitForTimeout(1200);
    await shot(guest.page, vp, '07-received');

    // A short real exchange so the feed has content.
    await utter(guest.page, 'está a diez minutos andando');
    await utter(host.page, 'is it far from here');
    await utter(guest.page, 'sigue recto y gira a la izquierda en la plaza');
    await utter(host.page, 'great thank you');
    await host.page.waitForTimeout(800);
    await shot(host.page, vp, '08-conversation');
    await shot(guest.page, vp, '08b-conversation-guest');

    await safe('settings', async () => {
      await openSettings(host.page);
      await shot(host.page, vp, '09-settings');
      await host.page.getByRole('dialog').evaluate((d) => d.querySelector('.scroll-area, [data-sheet-body]')?.scrollTo(0, 99999));
      await shot(host.page, vp, '09b-settings-scrolled');
      await host.page.getByRole('button', { name: /Where things run/ }).click();
      await shot(host.page, vp, '10-where-things-run');
      await host.page.getByRole('button', { name: 'Back' }).click();
      await safe('memory', async () => {
        await host.page.getByRole('button', { name: /What it remembers/ }).click({ timeout: 1500 });
        await shot(host.page, vp, '11-memory');
        await host.page.getByRole('button', { name: 'Back' }).click();
      });
      await host.page.getByRole('button', { name: /Your language/ }).click();
      await shot(host.page, vp, '12-language');
      await closeSheet(host.page);
    });

    await safe('hands-free', async () => {
      await openSettings(host.page);
      await host.page.getByRole('radio', { name: 'Hands-free' }).click();
      await closeSheet(host.page);
      await host.page.waitForFunction(() => window.__srRunning());
      await host.page.evaluate(() => window.__say('see you at the', false));
      await shot(host.page, vp, '13a-hands-free');
      await host.page.getByRole('button', { name: 'Stop hands-free' }).click();
    });

    await safe('typing', async () => {
      await host.page.getByRole('button', { name: 'Type' }).click({ timeout: 1500 });
      await host.page.getByRole('textbox', { name: 'Type a message' }).fill('See you there');
      await shot(host.page, vp, '13-typing');
      await host.page.getByRole('button', { name: 'Type' }).click();
    });

    await safe('reconnecting', async () => {
      await guest.page.evaluate(() => window.__dropSockets());
      await guest.page.waitForTimeout(600);
      await shot(guest.page, vp, '14-reconnecting');
      await shot(host.page, vp, '14b-peer-away');
    });

    await safe('ended', async () => {
      await guest.page.waitForTimeout(2500);
      await host.page.getByRole('button', { name: 'End call' }).click();
      await guest.page.waitForTimeout(400);
      await shot(guest.page, vp, '15-peer-left');
    });

    await host.context.close();
    await guest.context.close();
    console.log(`captured ${vp} (${scheme})`);
  }
} finally {
  await browser.close();
  await stack.stop();
}
