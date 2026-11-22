// Phase 7 accessibility checks (no extra dependencies):
//  1. WCAG contrast for every text/background token pair, both themes (computed from src/index.css)
//  2. Touch targets on a phone viewport (all visible interactive elements, key screens)
//  3. Keyboard: tab order on Home, sheet focus moves in / is trapped / returns, Escape closes,
//     Space push-to-talk
//  4. 200 % text size: no horizontal scroll on phone screens
//  5. prefers-reduced-motion: no running CSS animations or transitions longer than 0.01 s
// Usage: node e2e/a11y.mjs   (exit 1 on failure; prints a report)
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { loadChromium, startStack, initScript, root, readCallCode } from './harness.mjs';

const results = [];
async function step(name, fn) {
  try {
    const note = await fn();
    results.push({ name, ok: true });
    console.log(`  ✔ ${name}${note ? `\n      ${note}` : ''}`);
  } catch (err) {
    results.push({ name, ok: false });
    console.log(`  ✖ ${name}\n      ${String(err?.message || err).split('\n').join('\n      ')}`);
  }
}

// ---------------------------------------------------------------- 1. contrast
function tokens(css, light) {
  const block = light ? css.slice(css.indexOf('@media (prefers-color-scheme: light)')) : css.slice(css.indexOf(':root'));
  const body = block.slice(block.indexOf('{', block.indexOf(':root')) + 1);
  const out = {};
  for (const m of body.slice(0, body.indexOf('}')).matchAll(/--([a-z-]+):\s*(#[0-9a-f]{6})/gi)) out[m[1]] = m[2];
  return out;
}
const lum = (hex) => {
  const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
};
const ratio = (a, b) => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
};
const css = readFileSync(path.join(root, 'src/index.css'), 'utf8');
const dark = tokens(css, false);
const light = { ...dark, ...tokens(css, true) };
// [fg token, bg token, minimum] — 4.5 body text, 3.0 large text / UI components.
const PAIRS = [
  ['fg', 'bg', 4.5], ['fg', 'raised', 4.5], ['fg', 'surface', 4.5], ['fg', 'selected', 4.5],
  ['fg-muted', 'bg', 4.5], ['fg-muted', 'raised', 4.5], ['fg-muted', 'surface', 4.5],
  ['fg-subtle', 'bg', 4.5], ['fg-subtle', 'raised', 4.5], ['fg-subtle', 'surface', 4.5],
  ['on-accent', 'accent', 4.5], ['accent-text', 'bg', 4.5], ['accent-text', 'raised', 4.5],
  ['incoming', 'bg', 4.5], ['danger', 'bg', 4.5], ['danger', 'raised', 4.5],
  ['success', 'bg', 3], ['focus', 'bg', 3], ['focus', 'raised', 3], ['line-strong', 'bg', 1.5],
];
console.log('Accessibility checks');
for (const [name, t] of [['dark', dark], ['light', light]]) {
  await step(`contrast (${name} theme): ${PAIRS.length} token pairs meet their minimum`, async () => {
    const rows = PAIRS.map(([f, b, min]) => ({ pair: `${f} on ${b}`, ratio: ratio(t[f], t[b]), min }));
    const fails = rows.filter((r) => r.ratio < r.min);
    assert.deepEqual(fails.map((r) => `${r.pair} ${r.ratio.toFixed(2)} < ${r.min}`), []);
    const lowest = rows.filter((r) => r.min === 4.5).sort((a, b) => a.ratio - b.ratio)[0];
    return `lowest text pair: ${lowest.pair} ${lowest.ratio.toFixed(2)}:1`;
  });
}

// ---------------------------------------------------------------- browser checks
const stack = await startStack({ apiPort: 8971, webPort: 4271, outDir: path.join(root, 'e2e/.dist-a11y') });
const chromium = await loadChromium();
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });
const PHONE = { viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true };

async function ctx(locale, extra = {}) {
  const context = await browser.newContext({ locale, permissions: ['microphone'], ...extra });
  await context.addInitScript(initScript, {});
  return { context, page: await context.newPage() };
}
/** Interactive elements smaller than min × min (visible, enabled). */
const smallTargets = (page, min = 44) =>
  page.evaluate((min) => {
    const els = [...document.querySelectorAll('button, a[href], input, [role="switch"], [role="radio"], [tabindex="0"]')];
    return els
      .filter((e) => {
        const r = e.getBoundingClientRect();
        const s = getComputedStyle(e);
        return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && !e.disabled && e.getAttribute('aria-hidden') !== 'true';
      })
      .map((e) => {
        const r = e.getBoundingClientRect();
        return { label: e.getAttribute('aria-label') || e.textContent.trim().slice(0, 30) || e.tagName, w: Math.round(r.width), h: Math.round(r.height) };
      })
      .filter((t) => t.w < min || t.h < min);
  }, min);

let host, guest;
try {
  // Pair on phones, in a real call.
  host = await ctx('en-US', PHONE);
  guest = await ctx('es-ES', PHONE);
  const report = {};
  await host.page.goto(stack.web);
  report.home = await smallTargets(host.page);
  await host.page.getByLabel('Your name').fill('Ana');
  await host.page.getByRole('button', { name: 'Start a call' }).click();
  const code = await readCallCode(host.page);
  report.invite = await smallTargets(host.page);
  await guest.page.goto(`${stack.web}/?join=${code}`);
  report.join = await smallTargets(guest.page);
  await guest.page.getByLabel('Your name').fill('Luis');
  await guest.page.getByRole('button', { name: 'Join' }).click();
  await host.page.getByRole('dialog', { name: /before you talk/i }).waitFor({ timeout: 15000 });
  report.consent = await smallTargets(host.page);
  for (const p of [host.page, guest.page]) await p.getByRole('button', { name: /allow microphone/i }).click();
  await host.page.waitForTimeout(400);
  report.call = await smallTargets(host.page);
  await host.page.getByRole('button', { name: 'Call settings' }).click();
  await host.page.waitForTimeout(400);
  report.settings = await smallTargets(host.page);
  await host.page.keyboard.press('Escape');

  await step('touch targets ≥ 44 × 44 px on phone (home, invite, join, consent, call, settings)', async () => {
    const all = Object.entries(report).flatMap(([screen, list]) => list.map((t) => `${screen}: "${t.label}" ${t.w}×${t.h}`));
    assert.deepEqual(all, []);
  });

  await step('200 % text size: no horizontal scroll on the call and settings screens', async () => {
    await host.page.addStyleTag({ content: 'html { font-size: 200% !important; }' });
    const sw = () => host.page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    assert.ok((await sw()) <= 0, 'call screen scrolls horizontally');
    await host.page.getByRole('button', { name: 'Call settings' }).click();
    await host.page.waitForTimeout(300);
    assert.ok((await sw()) <= 0, 'settings scroll horizontally');
    // The talk control stays on screen.
    await host.page.keyboard.press('Escape');
    const box = await host.page.getByRole('button', { name: 'Push to talk' }).boundingBox();
    assert.ok(box && box.y + box.height <= 844, `talk button off screen at 200 %: ${JSON.stringify(box)}`);
  });
  await host.context.close();
  await guest.context.close();

  // Keyboard on desktop.
  const kb = await ctx('en-US', { viewport: { width: 1280, height: 800 } });
  const kg = await ctx('es-ES', { viewport: { width: 1280, height: 800 } });
  await step('keyboard: Home tab order is name → language → Start → Join', async () => {
    await kb.page.goto(stack.web);
    const order = [];
    for (let i = 0; i < 4; i++) {
      await kb.page.keyboard.press('Tab');
      order.push(await kb.page.evaluate(() => document.activeElement.getAttribute('aria-label') || document.activeElement.textContent.trim() || document.activeElement.id));
    }
    assert.deepEqual(order.map((o) => o.replace(/:.*/, '')), ['name', 'Your language', 'Start a call', 'Join with a code']);
  });
  await step('keyboard: whole call flow; sheet focus moves in, is trapped, Escape closes and focus returns', async () => {
    await kb.page.keyboard.press('Shift+Tab');
    await kb.page.keyboard.press('Shift+Tab');
    await kb.page.keyboard.press('Shift+Tab');
    await kb.page.keyboard.type('Ana');
    await kb.page.keyboard.press('Enter'); // submits "Start a call"
    const code = await readCallCode(kb.page);
    await kg.page.goto(`${stack.web}/?join=${code}`);
    await kg.page.getByLabel('Your name').fill('Luis');
    await kg.page.keyboard.press('Enter');
    const consent = kb.page.getByRole('dialog', { name: /before you talk/i });
    await consent.waitFor({ timeout: 15000 });
    assert.equal(await consent.evaluate((d) => d.contains(document.activeElement)), true, 'focus not moved into consent');
    assert.equal(await kb.page.evaluate(() => document.activeElement.textContent.trim()), 'Allow microphone');
    await kb.page.keyboard.press('Enter');
    await kg.page.getByRole('button', { name: /allow microphone/i }).click();
    const settingsBtn = kb.page.getByRole('button', { name: 'Call settings' });
    await settingsBtn.focus();
    await kb.page.keyboard.press('Enter');
    const sheet = kb.page.getByRole('dialog', { name: 'Call settings' });
    await sheet.waitFor();
    for (let i = 0; i < 40; i++) {
      await kb.page.keyboard.press('Tab');
      assert.equal(await sheet.evaluate((d) => d.contains(document.activeElement)), true, `focus escaped the sheet after ${i + 1} tabs`);
    }
    await kb.page.keyboard.press('Escape');
    await sheet.waitFor({ state: 'detached' });
    assert.equal(await kb.page.evaluate(() => document.activeElement.getAttribute('aria-label')), 'Call settings', 'focus did not return');
  });
  await step('keyboard: holding Space talks; releasing stops; ignored while typing', async () => {
    await kb.page.evaluate(() => document.activeElement.blur());
    await kb.page.keyboard.down('Space');
    await kb.page.waitForFunction(() => window.__srRunning(), null, { timeout: 3000 });
    assert.equal(await kb.page.getByRole('button', { name: 'Push to talk' }).getAttribute('aria-pressed'), 'true');
    await kb.page.keyboard.up('Space');
    await kb.page.waitForFunction(() => !window.__srRunning(), null, { timeout: 3000 });
    await kb.page.getByRole('button', { name: 'Type' }).click();
    await kb.page.getByRole('textbox', { name: 'Type a message' }).focus();
    await kb.page.keyboard.press('Space');
    assert.equal(await kb.page.evaluate(() => window.__srRunning()), false, 'Space in the text box started talking');
  });
  await step('the status line is a polite live region; the conversation is a labelled log', async () => {
    assert.equal(await kb.page.locator('[aria-live="polite"]').count() >= 1, true);
    assert.equal(await kb.page.getByRole('log', { name: 'Conversation' }).count(), 1);
    assert.equal(await kb.page.getByRole('main').count() + (await kb.page.locator('header').count()) >= 1, true);
  });
  await kb.context.close();
  await kg.context.close();

  await step('reduced motion: no animation or transition longer than 10 ms on Home and Invite', async () => {
    const rm = await ctx('en-US', { reducedMotion: 'reduce' });
    await rm.page.goto(stack.web);
    await rm.page.getByLabel('Your name').fill('Ana');
    await rm.page.getByRole('button', { name: 'Start a call' }).click();
    await readCallCode(rm.page);
    const long = await rm.page.evaluate(() =>
      [...document.querySelectorAll('*')]
        .map((e) => {
          const s = getComputedStyle(e);
          const secs = (v) => Math.max(...v.split(',').map((x) => parseFloat(x) * (x.includes('ms') ? 0.001 : 1)));
          return { tag: e.className?.toString?.().slice(0, 40), a: s.animationName !== 'none' ? secs(s.animationDuration) : 0, t: secs(s.transitionDuration) };
        })
        .filter((x) => x.a > 0.01 || x.t > 0.01)
    );
    await rm.context.close();
    assert.deepEqual(long, []);
  });
} finally {
  await browser.close();
  await stack.stop();
}
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed}/${results.length} accessibility checks passed`);
process.exit(failed ? 1 : 0);
