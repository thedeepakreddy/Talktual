// Phase 5 communication-intelligence E2E: two real Chromium pages, real
// signaling server (provider fetch stubbed — no live calls), default build
// (Phase 4 layer + ACI on, VoiceMirror off), then a rollback build with
// VITE_COMMUNICATION_INTELLIGENCE=0. Speech is scripted (no real voices).
//
// Usage: node e2e/intelligence.mjs

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
async function press(page) {
  await page.getByRole('button', { name: 'Push to talk' }).hover();
  await page.mouse.down();
  await page.waitForFunction(() => window.__srRunning());
}
const say = (page, text) => page.evaluate((t) => window.__say(t), text);
async function utter(page, text) {
  await press(page);
  await say(page, text);
  await page.mouse.up();
}
const spoken = (page) => page.evaluate(() => window.__spoken.map((s) => s.text));
const waitSpoken = (page, sub, ms = 8000) => page.waitForFunction((s) => window.__spoken.some((x) => x.text.includes(s)), sub, { timeout: ms });
async function transcript(page) {
  await page.getByRole('button', { name: 'Show transcript' }).click();
  const t = await page.getByRole('dialog', { name: 'Transcript' }).innerText();
  await page.getByRole('button', { name: 'Close transcript' }).click();
  return t;
}

const chromium = await loadChromium();
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });
let exitCode = 0;

const stack = await startStack({ apiPort: 8931, webPort: 4231, outDir: path.join(root, 'e2e/.dist-aci') });
let host, guest;
try {
  console.log(`Chromium ${browser.version()} — communication intelligence (default build)`);
  await step('pair; disclosure mentions drafts and on-device assistance; both accept', async () => {
    ({ host, guest } = await pairPeers(browser, stack.web));
    const dlg = host.page.getByRole('dialog', { name: /before you talk/i });
    const text = await dlg.innerText();
    assert.match(text, /draft translation/);
    assert.match(text, /runs on your device/);
    for (const p of [host.page, guest.page]) await p.getByRole('button', { name: /allow microphone/i }).click();
  });

  await step('context-assisted translation: name kept verbatim (never sent to the translator), sense from context, labelled', async () => {
    await utter(host.page, 'We went fishing by the river');
    await waitSpoken(guest.page, 'fishing');
    await utter(host.page, 'Tell Rose to wait at the bank');
    await waitSpoken(guest.page, 'Rose');
    const s = await spoken(guest.page);
    const line = s.find((t) => t.includes('Rose'));
    assert.ok(line.includes('riverbank'), `spoken: ${line}`);
    const q = stack.providerCalls.filter((c) => c.kind === 'translate').map((c) => c.q);
    assert.ok(q.some((x) => x === 'Tell QX0Q to wait at the riverbank'), JSON.stringify(q));
    assert.ok(!q.some((x) => x.includes('Rose')), 'protected name reached the translator');
    assert.match(await transcript(guest.page), /Their app used context/);
    assert.match(await transcript(host.page), /Context-assisted/);
  });

  await step('listener notes: reference resolved from earlier context; never spoken', async () => {
    await utter(host.page, 'Let us meet at Plaza Mayor tomorrow');
    await waitSpoken(guest.page, 'Plaza Mayor');
    await utter(guest.page, '¿Cómo llego allí desde el hotel?');
    await waitSpoken(host.page, 'hotel');
    const t = await transcript(host.page);
    assert.match(t, /“allí” → Plaza Mayor/);
    assert.ok(!(await spoken(host.page)).some((x) => x.includes('mentioned earlier')), 'a note was spoken');
  });

  await step('misunderstanding → dismissible suggestion for the speaker; nothing sent on their behalf', async () => {
    const before = stack.providerCalls.length;
    await utter(guest.page, '¿Cómo? No te entiendo');
    const note = host.page.getByRole('note', { name: 'Suggestion' });
    await note.waitFor({ timeout: 8000 });
    assert.match(await note.innerText(), /may not have understood/);
    await host.page.getByRole('button', { name: 'Dismiss suggestion' }).click();
    await note.waitFor({ state: 'detached', timeout: 3000 });
    // Only Luis's own phrase was translated — no automatic rephrase was sent.
    assert.equal(stack.providerCalls.length - before, 1);
  });

  await step('assist sheet: context visible and correctable; off switch clears it and translation becomes context-free', async () => {
    await host.page.getByRole('button', { name: 'Conversation assistance' }).click();
    const sheet = host.page.getByRole('dialog', { name: 'Conversation assistance' });
    assert.match(await sheet.innerText(), /Plaza Mayor/);
    await host.page.getByRole('switch', { name: 'Assistance' }).click();
    await host.page.getByText('Nothing yet.').waitFor();
    await host.page.getByRole('button', { name: 'Close assistance' }).click();
    await utter(host.page, 'Tell Rose to wait at the bank again');
    await waitSpoken(guest.page, 'again');
    const q = stack.providerCalls.filter((c) => c.kind === 'translate').map((c) => c.q);
    assert.ok(q.includes('Tell Rose to wait at the bank again'), 'expected the plain (context-free) text');
  });

  await step('no page errors; VoiceMirror off', async () => {
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

const off = await startStack({ apiPort: 8932, webPort: 4232, outDir: path.join(root, 'e2e/.dist-aci-off'), buildEnv: { VITE_COMMUNICATION_INTELLIGENCE: '0' } });
try {
  console.log('Rollback build (VITE_COMMUNICATION_INTELLIGENCE=0)');
  let h, g;
  await step('rollback: no assistance UI or disclosure point; translator receives the plain text', async () => {
    ({ host: h, guest: g } = await pairPeers(browser, off.web));
    const dlgText = await h.page.getByRole('dialog', { name: /before you talk/i }).innerText();
    assert.doesNotMatch(dlgText, /runs on your device/);
    for (const p of [h.page, g.page]) await p.getByRole('button', { name: /allow microphone/i }).click();
    assert.equal(await h.page.getByRole('button', { name: 'Conversation assistance' }).count(), 0);
    await utter(h.page, 'Tell Rose to wait at the bank');
    await waitSpoken(g.page, 'Rose');
    assert.ok(off.providerCalls.some((c) => c.q === 'Tell Rose to wait at the bank'));
    assert.ok(!off.providerCalls.some((c) => (c.q ?? '').includes('QX')));
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

console.log(`\n${results.filter((r) => r.ok).length}/${results.length} intelligence checks passed`);
process.exit(exitCode);
