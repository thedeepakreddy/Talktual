// Phase 7 final: real renders of every destination and key state.
// Usage: node e2e/final-screens.mjs <outDir> [phone,tablet,desktop] [dark|light] [scale]
import path from 'node:path';
import { mkdirSync } from 'node:fs';
import { loadChromium, startStack, initScript, root, readCallCode } from './harness.mjs';

const out = path.resolve(process.argv[2] || path.join(root, 'docs/phase7_final/after'));
const want = (process.argv[3] || 'phone,desktop').split(',');
const scheme = process.argv[4] === 'light' ? 'light' : 'dark';
const scale = Number(process.argv[5] || 1);
mkdirSync(out, { recursive: true });
const VP = { phone: { width: 390, height: 844 }, tablet: { width: 834, height: 1112 }, desktop: { width: 1440, height: 900 } };
const dict = {
  'hu|en': {
    'Elnézést, szabad ez a hely?': 'Excuse me, is this seat free?',
    'Igen, tessék, úgyis mindjárt megyünk.': 'Yes, go ahead, we are leaving soon anyway.',
    'Időben indul ma a bécsi vonat?': 'Is the train to Vienna on time today?',
  },
  'en|es': { 'where is the station': '¿Dónde está la estación?' },
  'es|en': { 'está a diez minutos andando': 'It is a ten-minute walk.' },
};
const stack = await startStack({ apiPort: 8991, webPort: 4291, outDir: path.join(root, 'e2e/.dist-final'), fetchOpts: { dict } });
const chromium = await loadChromium();
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });
const sfx = scheme === 'light' ? '-light' : '';
const shot = async (p, vp, name) => {
  await p.waitForTimeout(400);
  await p.screenshot({ path: path.join(out, `${vp}-${name}${sfx}.png`) });
};
const safe = async (label, fn) => {
  try {
    await fn();
  } catch (e) {
    console.log(`  (skipped ${label})`, String(e.message).split('\n')[0]);
  }
};
const ctx = async (vp, locale = 'en-US', opts = {}) => {
  const c = await browser.newContext({ locale, colorScheme: scheme, permissions: ['microphone'], viewport: VP[vp], deviceScaleFactor: scale, hasTouch: vp !== 'desktop', isMobile: vp === 'phone' });
  await c.addInitScript(initScript, opts);
  return c;
};
try {
  for (const vp of want) {
    const c = await ctx(vp);
    const p = await c.newPage();
    await p.goto(stack.web);
    await shot(p, vp, '01-home');
    for (const [r, n] of [['connect', '10-connect'], ['lens', '20-lens'], ['rewind', '21-rewind'], ['voice', '22-voice'], ['settings', '23-settings']]) {
      await p.goto(`${stack.web}/#/${r}`);
      await shot(p, vp, n);
    }
    await safe('settings-scrolled', async () => {
      await p.evaluate(() => window.scrollTo(0, 99999));
      await shot(p, vp, '23b-settings-scrolled');
    });
    // AIR
    await p.goto(`${stack.web}/#/air`);
    await shot(p, vp, '02-air-ready');
    await p.getByRole('button', { name: 'Start listening' }).click();
    await p.getByRole('dialog', { name: 'Listening to the room' }).waitFor();
    await shot(p, vp, '03-air-consent');
    await p.getByRole('dialog').getByRole('button', { name: 'Start listening' }).click();
    await p.waitForFunction(() => window.__srRunning());
    await shot(p, vp, '04-air-listening-empty');
    for (const t of Object.keys(dict['hu|en'])) {
      await p.waitForFunction(() => window.__srRunning(), null, { timeout: 5000 });
      await p.evaluate((x) => {
        window.__say(x.split(' ').slice(0, 2).join(' '), false);
      }, t);
      await p.waitForTimeout(300);
      await p.evaluate((x) => window.__say(x, true), t);
      await p.waitForTimeout(700);
    }
    await p.waitForFunction(() => window.__srRunning(), null, { timeout: 5000 });
    await p.evaluate(() => window.__say('Szerintem húsz percet késik, mert', false));
    await p.waitForTimeout(500);
    await p.evaluate(() => window.__say('Szerintem húsz percet késik, mert a', false));
    await p.waitForTimeout(250);
    await shot(p, vp, '05-air-active');
    await p.getByRole('button', { name: 'Pause listening' }).click();
    await shot(p, vp, '06-air-paused');
    await p.getByRole('button', { name: 'Stop listening' }).click();
    await shot(p, vp, '07-air-stopped');
    await safe('air-settings', async () => {
      await p.getByRole('button', { name: /settings/i }).first().click();
      await shot(p, vp, '08-air-settings');
      await p.getByRole('radio', { name: 'On-device' }).click();
      await p.keyboard.press('Escape');
      await p.getByRole('button', { name: /Start again|Start listening/ }).click();
      await shot(p, vp, '09-air-unavailable');
      await p.getByRole('button', { name: /settings/i }).first().click();
      await p.getByRole('radio', { name: 'Cloud' }).click();
      await p.keyboard.press('Escape');
    });
    await c.close();

    // Connect: invite, join, call
    await safe('connect', async () => {
      const hc = await ctx(vp);
      const gc = await ctx(vp, 'es-ES');
      const h = await hc.newPage();
      const g = await gc.newPage();
      await h.goto(`${stack.web}/#/connect`);
      await h.getByLabel('Your name').fill('Ana');
      await h.getByRole('button', { name: 'Start a call' }).click();
      const code = await readCallCode(h);
      await shot(h, vp, '11-connect-invite');
      await g.goto(`${stack.web}/?join=${code}`);
      await shot(g, vp, '12-connect-join');
      await g.getByLabel('Your name').fill('Luis');
      await g.getByRole('button', { name: /^join$/i }).click();
      await h.getByRole('button', { name: 'Push to talk' }).waitFor({ timeout: 15000 });
      for (const pg of [h, g]) await pg.getByRole('button', { name: /allow microphone/i }).click();
      await shot(h, vp, '13-call-idle');
      const say = async (pg, t) => {
        await pg.getByRole('button', { name: 'Push to talk' }).hover();
        await pg.mouse.down();
        await pg.waitForFunction(() => window.__srRunning());
        await pg.evaluate((x) => window.__say(x), t);
        await pg.mouse.up();
        await pg.waitForTimeout(900);
      };
      await say(h, 'where is the station');
      await say(g, 'está a diez minutos andando');
      await shot(h, vp, '14-call-conversation');
      await shot(g, vp, '14b-call-conversation-guest');
      await hc.close();
      await gc.close();
    });
    console.log(`captured ${vp} (${scheme})`);
  }
} finally {
  await browser.close();
  await stack.stop();
}
