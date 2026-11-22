// Recovery E2E on the relay-only path (UDP disabled, so no P2P data channel):
// losing the signaling socket really does interrupt the call here.
//
//   A. blip: host's socket is cut → host reconnects with its resume token →
//      call continues, the next turn is heard exactly once.
//   B. outage: host goes offline past the seat-hold grace → guest is told the
//      peer left; when the host comes back its resume is refused and it is
//      told the call ended while it was offline.
//
// Usage: node e2e/recovery.mjs

import assert from 'node:assert/strict';
import { loadChromium, startStack, pairPeers, talk, readTransport } from './harness.mjs';

const GRACE_MS = 4000;
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

const stack = await startStack({ apiPort: 8913, webPort: 4213, serverOverrides: { resumeGraceMs: GRACE_MS } });
const chromium = await loadChromium();
const browser = await chromium.launch({
  args: [
    '--use-fake-ui-for-media-stream',
    '--use-fake-device-for-media-stream',
    // No UDP → ICE can't connect → relay transport only.
    '--force-webrtc-ip-handling-policy=disable_non_proxied_udp',
  ],
});
let exitCode = 0;
const seats = () => [...stack.api.sessions.values()].flatMap((s) => s.seats);

async function consent(p) {
  await p.getByRole('button', { name: /allow microphone/i }).click();
}

try {
  console.log(`Chromium ${browser.version()} — relay-only recovery scenarios`);

  // ---------------- A ----------------
  let pair;
  await step('A: call established on the relay (no P2P)', async () => {
    pair = await pairPeers(browser, stack.web);
    await consent(pair.host.page);
    await consent(pair.guest.page);
    await pair.host.page.waitForTimeout(3000);
    const t = await readTransport(pair.host.page);
    assert.equal(t, 'Relayed');
    await talk(pair.host.page, 'before');
    await pair.guest.page.waitForFunction(() => window.__spoken.some((s) => s.text === '[en|es] before'));
  });

  await step('A: socket cut → guest told the peer is away; host shows reconnecting then recovers', async () => {
    await pair.host.page.evaluate(() => window.__dropSockets());
    await pair.guest.page.getByText(/lost connection — waiting for them/i).waitFor({ timeout: 5000 });
    await pair.host.page.getByText(/reconnect/i).first().waitFor({ timeout: 5000 });
    await pair.host.page.getByText('Hold to talk').waitFor({ timeout: 10000 });
    assert.equal(seats().filter((s) => s.ws && s.ws.readyState === 1).length, 2);
  });

  await step('A: next turn is heard exactly once after recovery', async () => {
    const before = await pair.guest.page.evaluate(() => window.__spoken.length);
    await talk(pair.host.page, 'after');
    await pair.guest.page.waitForFunction(() => window.__spoken.some((s) => s.text === '[en|es] after'), null, { timeout: 10000 });
    await pair.guest.page.waitForTimeout(800);
    const spoken = await pair.guest.page.evaluate(() => window.__spoken.map((s) => s.text));
    assert.equal(spoken.filter((t) => t === '[en|es] after').length, 1);
    assert.equal(spoken.filter((t) => t === '[en|es] before').length, 1, 'earlier turn not replayed');
    assert.equal(spoken.length, before + 1);
  });

  await step('A: no page errors', async () => {
    assert.deepEqual([...pair.host.errors, ...pair.guest.errors], []);
  });

  await pair.host.context.close();
  await pair.guest.context.close();
  await new Promise((r) => setTimeout(r, 300));

  // ---------------- B ----------------
  let p2;
  await step('B: new call on the relay', async () => {
    p2 = await pairPeers(browser, stack.web);
    await consent(p2.host.page);
    await consent(p2.guest.page);
  });

  await step(`B: host offline past the ${GRACE_MS / 1000}s grace → guest is told they left and returns home`, async () => {
    await p2.host.context.setOffline(true);
    await p2.host.page.evaluate(() => window.__dropSockets());
    await p2.guest.page.getByText(/lost connection — waiting for them/i).waitFor({ timeout: 5000 });
    await p2.guest.page.getByText(/left the call/i).first().waitFor({ timeout: GRACE_MS + 5000 });
    await p2.guest.page.getByRole('button', { name: /^start a call$/i }).waitFor({ timeout: 5000 });
  });

  await step('B: host back online → resume refused → "call ended while you were offline"', async () => {
    await p2.host.context.setOffline(false);
    await p2.host.page.getByText(/ended while you were offline|disconnected/i).first().waitFor({ timeout: 30000 });
    const msg = await p2.host.page.getByRole('alert').first().textContent();
    console.log(`      host notice: "${msg}"`);
  });

  await step('B: server holds no sessions afterwards', async () => {
    await p2.host.page.getByRole('button', { name: 'End call' }).click();
    await new Promise((r) => setTimeout(r, 300));
    assert.equal(stack.api.sessions.size, 0);
  });
} catch {
  exitCode = 1;
} finally {
  await browser.close();
  await stack.stop();
  console.log(`\n${results.filter((r) => r.ok).length}/${results.length} recovery steps passed`);
  process.exit(exitCode);
}
