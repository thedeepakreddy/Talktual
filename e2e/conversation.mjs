// Phase 4 conversation E2E: two real Chromium pages, real signaling server
// (in-process, provider calls stubbed), production build with the default
// flags (conversation layer ON, VoiceMirror OFF). Speech recognition and
// synthesis are scripted fakes (no real speech), so this checks behaviour and
// wiring, not recognition quality or real-world timing.
// A second build with VITE_CONVERSATION_LAYER=0 checks the rollback path.
//
// Usage: node e2e/conversation.mjs

import assert from 'node:assert/strict';
import path from 'node:path';
import { loadChromium, startStack, pairPeers, root } from './harness.mjs';

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

const spokenTexts = (page) => page.evaluate(() => window.__spoken.map((s) => s.text));
async function press(page) {
  await page.getByRole('button', { name: 'Push to talk' }).hover();
  await page.mouse.down();
  await page.waitForFunction(() => window.__srRunning());
}
const say = (page, text, isFinal = true) => page.evaluate(([t, f]) => window.__say(t, f), [text, isFinal]);
const release = (page) => page.mouse.up();
async function transcript(page) {
  await page.getByRole('button', { name: 'Show transcript' }).click();
  const text = await page.getByRole('dialog', { name: 'Transcript' }).innerText();
  await page.getByRole('button', { name: 'Close transcript' }).click();
  return text;
}
async function waitSpoken(page, pred, ms = 8000) {
  await page.waitForFunction((src) => window.__spoken.some((s) => new Function('t', `return ${src}`)(s.text)), pred, { timeout: ms });
}

const chromium = await loadChromium();
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });
let exitCode = 0;

// ---------------------------------------------------------------- layer ON
const stack = await startStack({ apiPort: 8921, webPort: 4221, outDir: path.join(root, 'e2e/.dist-conv') });
let host, guest;
try {
  console.log(`Chromium ${browser.version()} — conversation layer (default build)`);
  await step('pair; both accept microphone consent', async () => {
    ({ host, guest } = await pairPeers(browser, stack.web));
    for (const p of [host.page, guest.page]) await p.getByRole('button', { name: /allow microphone/i }).click();
  });

  await step('hesitation: held fragment + continuation are spoken ONCE as one phrase; a provisional was translated', async () => {
    await press(host.page);
    await say(host.page, 'i would like to book a');
    await host.page.waitForTimeout(600); // the speaker pauses mid-thought
    await say(host.page, 'table', false);
    await host.page.waitForTimeout(200);
    await say(host.page, 'table for two');
    await release(host.page);
    await waitSpoken(guest.page, "t.includes('i would like to book a table for two')");
    await guest.page.waitForTimeout(800);
    const spoken = await spokenTexts(guest.page);
    assert.equal(spoken.filter((t) => t.includes('book a')).length, 1, `spoken: ${JSON.stringify(spoken)}`);
    assert.ok(!spoken.some((t) => /book a$/.test(t)), 'fragment spoken on its own');
    const qs = stack.providerCalls.filter((c) => c.kind === 'translate').map((c) => c.q);
    assert.ok(qs.includes('i would like to book a'), 'provisional translation request expected');
    assert.ok(qs.includes('i would like to book a table for two'));
  });

  await step('self-repair: restart and repetition never reach the listener', async () => {
    await press(host.page);
    await say(host.page, 'i want to');
    await say(host.page, 'i want to go to the airport');
    await say(host.page, 'can you can you call a taxi');
    await release(host.page);
    await waitSpoken(guest.page, "t.includes('can you call a taxi')");
    await guest.page.waitForTimeout(500);
    const spoken = await spokenTexts(guest.page);
    assert.ok(spoken.some((t) => t.includes('i want to go to the airport')));
    assert.ok(!spoken.some((t) => /i want to$/.test(t)), 'abandoned start was spoken');
    assert.ok(!spoken.some((t) => t.includes('can you can you')), 'repetition was spoken');
  });

  await step('interruption: listener cuts in while the speaker holds the floor → rest of that turn is shown, not spoken', async () => {
    await press(host.page);
    await say(host.page, 'first let me explain the plan');
    await waitSpoken(guest.page, "t.includes('first let me explain the plan')");
    await press(guest.page); // Luis cuts in while Ana is still holding
    await say(guest.page, 'espera');
    await release(guest.page);
    await host.page.waitForTimeout(300);
    await say(host.page, 'and then the stale continuation');
    await release(host.page);
    await guest.page.waitForTimeout(2000);
    const spoken = await spokenTexts(guest.page);
    assert.ok(!spoken.some((t) => t.includes('stale continuation')), 'stale continuation was spoken');
    const gt = await transcript(guest.page);
    assert.match(gt, /stale continuation/);
    assert.match(gt, /Interrupted — not played/);
    assert.match(await transcript(host.page), /Interrupted/);
    // Luis's own words still reached Ana.
    await waitSpoken(host.page, "t.includes('espera')");
  });

  await step('overlap: speech arriving while the listener talks waits (chip shown), then plays after release', async () => {
    await press(guest.page);
    await say(guest.page, 'creo que', false);
    await press(host.page);
    await say(host.page, 'are you coming to dinner tonight');
    await release(host.page);
    await guest.page.getByText(/1 message waits until you finish/).waitFor({ timeout: 8000 });
    assert.ok(!(await spokenTexts(guest.page)).some((t) => t.includes('dinner tonight')), 'played over the listener');
    await say(guest.page, 'creo que sí');
    await release(guest.page);
    await waitSpoken(guest.page, "t.includes('are you coming to dinner tonight')");
  });

  await step('hands-free: an unfinished phrase is held, then committed when the speaker stops (bounded)', async () => {
    await host.page.getByRole('button', { name: /hands-free: off/i }).click();
    await host.page.waitForFunction(() => window.__srRunning(), null, { timeout: 5000 });
    const before = (await spokenTexts(guest.page)).length;
    await say(host.page, 'meet me at the');
    await guest.page.waitForTimeout(400);
    assert.equal((await spokenTexts(guest.page)).length, before, 'fragment committed too early');
    await waitSpoken(guest.page, "t.includes('meet me at the')", 6000);
    await host.page.getByRole('button', { name: /hands-free: on/i }).click();
  });

  await step('no page errors; VoiceMirror stayed off (default build)', async () => {
    for (const p of [host, guest]) assert.deepEqual(p.errors, []);
    assert.equal(await host.page.getByRole('button', { name: /voice match/i }).count(), 0);
  });
} catch {
  exitCode = 1;
} finally {
  await host?.context.close().catch(() => {});
  await guest?.context.close().catch(() => {});
  await stack.stop();
}

// ---------------------------------------------------------------- layer OFF (rollback)
const off = await startStack({
  apiPort: 8922,
  webPort: 4222,
  outDir: path.join(root, 'e2e/.dist-conv-off'),
  buildEnv: { VITE_CONVERSATION_LAYER: '0' },
});
try {
  console.log('Rollback build (VITE_CONVERSATION_LAYER=0)');
  let h, g;
  await step('rollback: Phase 3 behaviour — each recognizer final is its own segment', async () => {
    ({ host: h, guest: g } = await pairPeers(browser, off.web));
    for (const p of [h.page, g.page]) await p.getByRole('button', { name: /allow microphone/i }).click();
    await press(h.page);
    await say(h.page, 'i would like to book a');
    await h.page.waitForTimeout(600);
    await say(h.page, 'table for two');
    await release(h.page);
    await waitSpoken(g.page, "t.includes('table for two')");
    const spoken = await spokenTexts(g.page);
    assert.ok(spoken.some((t) => /book a$/.test(t)), `expected the split Phase 3 behaviour, got ${JSON.stringify(spoken)}`);
    assert.ok(!off.providerCalls.some((c) => c.q === 'i would like to book a table for two'));
    for (const p of [h, g]) assert.deepEqual(p.errors, []);
  });
  await h?.context.close().catch(() => {});
  await g?.context.close().catch(() => {});
} catch {
  exitCode = 1;
} finally {
  await off.stop();
  await browser.close();
}

const passed = results.filter((r) => r.ok).length;
console.log(`\n${passed}/${results.length} conversation checks passed`);
process.exit(exitCode);
