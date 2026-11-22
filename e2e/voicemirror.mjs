// VoiceMirror E2E (Phase 3): real browser path with the feature flag ON.
//
// Chromium's fake microphone plays a SYNTHETIC espeak-ng voice (no real person),
// so the page's real getUserMedia → ScriptProcessor → analyzer runs on speech.
// Speech recognition / synthesis are scripted fakes as in the other suites;
// synthesis records the pitch/rate each utterance was given.
//
// Usage: node e2e/voicemirror.mjs

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { loadChromium, startStack, pairPeers, talk, root, openSettings, closeSettings } from './harness.mjs';

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

// Synthetic "speaker": a low-pitched espeak voice, looped by Chromium as the mic.
const micWav = path.join(root, 'e2e/.dist-latency-mic.wav');
execFileSync('espeak-ng', ['-v', 'en-us+m3', '-p', '25', '-s', '150', '-w', micWav,
  'This is a synthetic test voice for the voice mirror check. It keeps talking for a few seconds so the analyzer can measure it.']);

const stack = await startStack({ apiPort: 8917, webPort: 4217, outDir: path.join(root, 'e2e/.dist-vm'), buildEnv: { VITE_VOICEMIRROR: '1' } });
const chromium = await loadChromium();
const browser = await chromium.launch({
  args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream', `--use-file-for-fake-audio-capture=${micWav}`],
});
let exitCode = 0;
let host, guest;
async function holdAndTalk(page, text, ms) {
  const ptt = page.getByRole('button', { name: 'Push to talk' });
  await ptt.hover();
  await page.mouse.down();
  await page.waitForFunction(() => window.__srRunning());
  await page.waitForTimeout(ms); // the fake mic keeps playing the synthetic voice
  await page.evaluate((t) => window.__say(t), text);
  await page.mouse.up();
}
const lastFor = (page, text) => page.evaluate((t) => window.__spoken.filter((s) => s.text === t).at(-1) ?? null, text);

try {
  console.log(`Chromium ${browser.version()} — VoiceMirror (flag on)`);
  await step('pair; both accept microphone consent; voice match is offered but OFF', async () => {
    ({ host, guest } = await pairPeers(browser, stack.web));
    for (const p of [host.page, guest.page]) await p.getByRole('button', { name: /allow microphone/i }).click();
    await openSettings(host.page);
    await host.page.getByRole('switch', { name: 'Voice match', checked: false }).waitFor();
    await closeSettings(host.page);
  });

  await step('default: speech is played with default pitch/rate (no voice data shared)', async () => {
    await talk(host.page, 'first turn');
    await guest.page.waitForFunction(() => window.__spoken.some((s) => s.text === '[en|es] first turn'));
    const u = await lastFor(guest.page, '[en|es] first turn');
    assert.equal(u.pitch, 1);
    assert.equal(u.rate, 1);
  });

  await step('opt-in shows the VoiceMirror disclosure; accept turns it on', async () => {
    await openSettings(host.page);
    await host.page.getByRole('switch', { name: 'Voice match' }).click();
    const d = host.page.getByRole('dialog', { name: /share your voice style/i });
    await d.waitFor();
    assert.match(await d.textContent(), /does not copy or clone/);
    await d.getByRole('button', { name: 'Share voice style' }).click();
    await d.waitFor({ state: 'detached' });
    await openSettings(host.page);
    await host.page.getByRole('switch', { name: 'Voice match', checked: true }).waitFor();
    await closeSettings(host.page);
  });

  let matched;
  await step('first short turn: not enough voiced speech yet → still default (fails closed)', async () => {
    await holdAndTalk(host.page, 'second turn', 3000);
    await guest.page.waitForFunction(() => window.__spoken.some((s) => s.text === '[en|es] second turn'), null, { timeout: 10000 });
    assert.equal(await guest.page.getByTestId('call-meta').getByText(/Voice matched/).count(), 0);
  });

  await step('host keeps talking (real mic → analyzer accumulates) → guest renders with the host’s voice style', async () => {
    await holdAndTalk(host.page, 'second and a half', 4000);
    await guest.page.getByTestId('call-meta').getByText(/Voice matched/).waitFor({ timeout: 10000 });
    await holdAndTalk(host.page, 'third turn', 500);
    await guest.page.waitForFunction(() => window.__spoken.some((s) => s.text === '[en|es] third turn'), null, { timeout: 10000 });
    matched = await lastFor(guest.page, '[en|es] third turn');
    console.log(`      guest utterance: pitch=${matched.pitch} rate=${matched.rate} (low synthetic voice → pitch < 1 expected)`);
    assert.ok(matched.pitch < 1, 'low voice renders below default pitch');
    assert.ok(matched.rate >= 0.8 && matched.rate <= 1.25);
  });

  await step('revocation: host turns it off → guest’s next segment is back to default', async () => {
    await openSettings(host.page);
    await host.page.getByRole('switch', { name: 'Voice match' }).click();
    await closeSettings(host.page);
    await guest.page.getByTestId('call-meta').getByText(/Voice matched/).waitFor({ state: 'detached', timeout: 5000 });
    await talk(host.page, 'fourth turn');
    await guest.page.waitForFunction(() => window.__spoken.some((s) => s.text === '[en|es] fourth turn'), null, { timeout: 10000 });
    const u = await lastFor(guest.page, '[en|es] fourth turn');
    assert.equal(u.pitch, 1);
    assert.equal(u.rate, 1);
  });

  await step('segments and transcript unaffected; no page errors', async () => {
    const spoken = await guest.page.evaluate(() => window.__spoken.map((s) => s.text));
    for (const t of ['first', 'second', 'third', 'fourth']) assert.equal(spoken.filter((x) => x === `[en|es] ${t} turn`).length, 1, t);
    assert.equal(spoken.filter((x) => x === '[en|es] second and a half').length, 1);
    assert.deepEqual([...host.errors, ...guest.errors], []);
  });

  await step('leave → server holds no sessions', async () => {
    await host.page.getByRole('button', { name: 'End call' }).click();
    await new Promise((r) => setTimeout(r, 500));
    assert.equal(stack.api.sessions.size, 0);
  });
} catch {
  exitCode = 1;
} finally {
  await browser.close();
  await stack.stop();
  console.log(`\n${results.filter((r) => r.ok).length}/${results.length} VoiceMirror e2e steps passed`);
  process.exit(exitCode);
}
