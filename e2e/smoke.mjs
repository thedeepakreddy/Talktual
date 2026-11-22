// End-to-end smoke test: two real Chromium contexts, the production build and
// the real server, with only external edges faked (translation provider,
// speech recognizer, speech synthesis). No live provider calls.
//
// Usage: npm run test:e2e

import assert from 'node:assert/strict';
import { loadChromium, startStack, pairPeers, talk, openSettings, closeSettings, feedText, readTransport, endCall } from './harness.mjs';

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

const stack = await startStack({
  apiPort: Number(process.env.E2E_API_PORT || 8899),
  webPort: Number(process.env.E2E_WEB_PORT || 4199),
  serverOverrides: { resumeGraceMs: 15000 },
  fetchOpts: { dict: { 'en|es': { 'hello there': 'hola ahí', 'how are you': 'cómo estás' }, 'es|en': { gracias: 'thanks' } } },
});
const chromium = await loadChromium();
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });
let exitCode = 0;
let host, guest;

try {
  console.log(`Chromium ${browser.version()} — running scenarios`);

  await step('host starts a call; guest joins via ?join= link; both land in the call', async () => {
    ({ host, guest } = await pairPeers(browser, stack.web));
    await guest.page.getByText('English').first().waitFor({ timeout: 5000 });
  });

  await step('P1: before consent, nothing is listening and the disclosure is shown', async () => {
    for (const p of [host.page, guest.page]) {
      await p.getByRole('dialog', { name: /before you talk/i }).waitFor({ timeout: 5000 });
      assert.equal(await p.evaluate(() => window.__srRunning()), false);
      assert.equal(await p.evaluate(() => window.__sr.length), 0, 'no recognizer even constructed');
    }
  });

  await step('P1: after consent, still nothing listens until the button is held', async () => {
    for (const p of [host.page, guest.page]) await p.getByRole('button', { name: /allow microphone/i }).click();
    await host.page.waitForTimeout(1500);
    assert.equal(await host.page.evaluate(() => window.__srRunning()), false);
    assert.equal(await guest.page.evaluate(() => window.__srRunning()), false);
  });

  let transport = 'unknown';
  await step('transport is reported (P2P data channel or relay)', async () => {
    transport = await readTransport(host.page);
    assert.ok(['Direct', 'Relayed'].includes(transport));
  });

  await step('host speaks EN → guest sees and hears ES translation', async () => {
    await talk(host.page, 'hello there');
    await guest.page.getByText('hola ahí').first().waitFor({ timeout: 10000 });
    await guest.page.waitForFunction(() => window.__spoken.some((s) => s.text === 'hola ahí' && s.lang === 'es-ES'));
  });

  await step('P1: the recognizer stops after release (push-to-talk stop semantics)', async () => {
    await host.page.waitForFunction(() => !window.__srRunning(), null, { timeout: 3000 });
    const log = await host.page.evaluate(() => window.__srLog.map((e) => e.ev));
    assert.deepEqual(log.slice(0, 2), ['start', 'stop']);
  });

  await step('guest speaks ES → host hears EN translation', async () => {
    await talk(guest.page, 'gracias');
    await host.page.getByText('thanks').first().waitFor({ timeout: 10000 });
    await host.page.waitForFunction(() => window.__spoken.some((s) => s.text === 'thanks' && s.lang === 'en-US'));
  });

  await step('multi-segment turn arrives in spoken order', async () => {
    const ptt = host.page.getByRole('button', { name: 'Push to talk' });
    await ptt.hover();
    await host.page.mouse.down();
    await host.page.waitForFunction(() => window.__srRunning());
    await host.page.evaluate(() => {
      window.__say('how are you');
      window.__say('hello there friend');
    });
    await host.page.mouse.up();
    await guest.page.waitForFunction(
      () => window.__spoken.filter((s) => s.text === 'cómo estás' || s.text.includes('hello there friend')).length === 2
    );
    const order = await guest.page.evaluate(() =>
      window.__spoken.map((s) => s.text).filter((t) => t === 'cómo estás' || t.includes('hello there friend'))
    );
    assert.deepEqual(order, ['cómo estás', '[en|es] hello there friend']);
  });

  await step('DEF-C02: translation failure is surfaced, not disguised', async () => {
    await talk(host.page, 'fail me');
    await host.page.getByText(/your words were sent untranslated/i).waitFor({ timeout: 10000 });
    await guest.page.getByText(/playing their original words/i).waitFor({ timeout: 10000 });
    await guest.page.waitForFunction(() => window.__spoken.some((s) => s.text === 'fail me' && s.lang === 'en-US'));
  });

  await step('hands-free: opt-in shows a persistent indicator, listens, and Stop ends it', async () => {
    await openSettings(host.page);
    await host.page.getByRole('radio', { name: 'Hands-free' }).click();
    await closeSettings(host.page);
    await host.page.getByText('Hands-free: listening continuously').waitFor({ timeout: 5000 });
    await host.page.waitForFunction(() => window.__srRunning());
    await host.page.evaluate(() => window.__say('hello there'));
    await guest.page.waitForFunction(() => window.__spoken.filter((s) => s.text === 'hola ahí').length === 2);
    await host.page.getByRole('button', { name: 'Stop', exact: true }).click();
    await host.page.waitForFunction(() => !window.__srRunning(), null, { timeout: 3000 });
  });

  await step('mid-call language switch retargets the next turn without reconnecting', async () => {
    await openSettings(guest.page);
    await guest.page.getByRole('button', { name: /your language: español/i }).click();
    await guest.page.getByLabel('Search languages').fill('French');
    await guest.page.getByRole('button', { name: /Français/ }).click();
    await host.page.getByText('Français').first().waitFor({ timeout: 5000 });
    await talk(host.page, 'hello there');
    await guest.page.waitForFunction(() => window.__spoken.some((s) => s.text === '[en|fr] hello there' && s.lang === 'fr-FR'));
  });

  await step('recovery: host signaling socket dropped → reconnects, call continues, nothing spoken twice', async () => {
    const before = await guest.page.evaluate(() => window.__spoken.length);
    // Cut the host's signaling socket (simulates a network blip on the host).
    await host.page.evaluate(() => window.__dropSockets());
    // P2P may carry the call through the blip; either way the next turn arrives once.
    await host.page.waitForTimeout(2500);
    await talk(host.page, 'after the blip');
    await guest.page.waitForFunction(() => window.__spoken.some((s) => s.text === '[en|fr] after the blip'), null, { timeout: 15000 });
    await guest.page.waitForTimeout(800);
    const after = await guest.page.evaluate(() => window.__spoken.slice());
    assert.equal(after.filter((s) => s.text === '[en|fr] after the blip').length, 1);
    assert.equal(after.length, before + 1, 'no duplicate or revived speech');
    // Signaling recovered: the host holds a live seat again.
    const live = [...stack.api.sessions.values()].flatMap((s) => s.seats).filter((s) => s.ws && s.ws.readyState === 1);
    assert.equal(live.length, 2, 'both seats live after reconnect');
  });

  await step('the conversation lists both sides, with honest translation state', async () => {
    const text = await feedText(host.page);
    assert.match(text, /hello there/);
    assert.match(text, /Luis/);
    assert.match(text, /Not translated — translation was unavailable/);
  });

  await step('host leaves → guest is told immediately and returned home', async () => {
    assert.equal(await endCall(host.page), true, 'a live call asks before ending');
    await host.page.getByRole('button', { name: /^start a call$/i }).waitFor();
    await guest.page.getByText(/left the call/i).first().waitFor({ timeout: 5000 });
    await guest.page.getByRole('button', { name: /^start a call$/i }).waitFor({ timeout: 10000 });
  });

  await step('server cleaned up every session after the call', async () => {
    await new Promise((r) => setTimeout(r, 300));
    assert.equal(stack.api.sessions.size, 0, `sessions left: ${[...stack.api.sessions.keys()]}`);
  });

  await step('DEF-C04: joining an unknown code fails with a message (no infinite spinner)', async () => {
    await guest.page.goto(`${stack.web}/?join=QQQQQQ`);
    await guest.page.getByLabel('Your name').fill('Luis');
    await guest.page.getByRole('button', { name: /^join$/i }).click();
    await guest.page.getByRole('alert').getByText(/no one is waiting/i).waitFor({ timeout: 20000 });
    await new Promise((r) => setTimeout(r, 300));
    assert.equal(stack.api.sessions.size, 0, 'abandoned lobby seat was released');
  });

  await step('no uncaught page errors in either browser', async () => {
    assert.deepEqual([...host.errors, ...guest.errors], []);
  });

  console.log(`\nTransport observed: ${transport}. Provider calls (stubbed): ${stack.providerCalls.length}.`);
} catch {
  exitCode = 1;
} finally {
  await browser.close();
  await stack.stop();
  console.log(`\n${results.filter((r) => r.ok).length}/${results.length} e2e steps passed`);
  process.exit(exitCode);
}
