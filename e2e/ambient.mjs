// Phase 8 — ambient translation, end to end in real Chromium against the real
// Talktual server (providers stubbed; translation goes through the server's
// MyMemory route to the stub). SIMULATED: the recognizer is a scripted Web
// Speech fake and speech output is a fake speechSynthesis. This proves the
// app's orchestration, policy and UI — not recognition accuracy, routing to
// AirPods, or café-grade performance.
// Usage: node e2e/ambient.mjs   (writes docs/phase8/results/ambient-e2e.json)
import assert from 'node:assert/strict';
import path from 'node:path';
import { mkdirSync, writeFileSync } from 'node:fs';
import { loadChromium, startStack, initScript, root } from './harness.mjs';

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

const PHRASES = [
  ['Jó napot kívánok', 'Good afternoon'],
  ['Hol van a pályaudvar', 'Where is the railway station'],
  ['Egy kávét kérek tejjel', 'A coffee with milk, please'],
  ['Mennyibe kerül a jegy', 'How much is the ticket'],
  ['Holnap esni fog az eső', 'It will rain tomorrow'],
  ['A vonat tíz perc múlva indul', 'The train leaves in ten minutes'],
  ['Köszönöm szépen a segítséget', 'Thank you very much for the help'],
  ['Szeretnék egy asztalt két főre', 'I would like a table for two'],
];
const dict = { 'hu|en': Object.fromEntries(PHRASES), 'en|hu': {} };
const stack = await startStack({ apiPort: 8981, webPort: 4281, outDir: path.join(root, 'e2e/.dist-ambient'), fetchOpts: { dict } });
const chromium = await loadChromium();
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });
const out = { simulated: true, note: 'Scripted recognizer + fake speech synthesis + stubbed provider via the real server. Not real-device or real-audio evidence.', latency: {}, steps: [] };
let exitCode = 0;
const pct = (a, p) => {
  const s = [...a].sort((x, y) => x - y);
  return s[Math.min(s.length - 1, Math.floor((p / 100) * s.length))];
};

try {
  console.log(`Chromium ${browser.version()} — ambient translation (simulated recognizer)`);
  const context = await browser.newContext({ locale: 'en-US', permissions: ['microphone'], viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
  await context.addInitScript(initScript, {});
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  const inferenceRequests = [];
  page.on('request', (r) => {
    if (/\/api\/(translate|tts|stt)/.test(r.url())) inferenceRequests.push(r.url());
  });

  await step('Home → "Translate what’s around you" opens setup; nothing is listening yet', async () => {
    await page.goto(stack.web);
    await page.getByRole('button', { name: /translate what’s around you/i }).click();
    await page.getByRole('heading', { name: 'Live translate' }).waitFor();
    await page.getByRole('button', { name: /They speak: Magyar/ }).waitFor();
    assert.equal(await page.evaluate(() => window.__sr.length), 0);
    assert.equal(await page.getByRole('button', { name: 'Start listening' }).count(), 1);
  });

  await step('Start → ambient disclosure first; no recognizer before Allow', async () => {
    await page.getByRole('button', { name: 'Start listening' }).click();
    const d = page.getByRole('dialog', { name: 'Listening to the room' });
    await d.waitFor();
    assert.match(await d.innerText(), /people near you/);
    assert.match(await d.innerText(), /doesn’t record or store audio/);
    assert.equal(await page.evaluate(() => window.__sr.length), 0);
    await d.getByRole('button', { name: 'Start listening' }).click();
    await page.waitForFunction(() => window.__srRunning());
    assert.equal(await page.evaluate(() => window.__sr.at(-1).lang), 'hu-HU');
    assert.equal(await page.evaluate(() => window.__sr.at(-1).continuous), true);
    await page.getByRole('button', { name: 'Pause listening' }).waitFor();
    assert.match(await page.title(), /Listening/);
    assert.match(await page.getByTestId('ambient-meta').innerText(), /Listening/);
  });

  const textMs = [];
  const speechMs = [];
  await step(`${PHRASES.length} Hungarian phrases → English text and speech, in order, once each (half-duplex: no headphones)`, async () => {
    for (const [hu, en] of PHRASES) {
      await page.waitForFunction(() => window.__srRunning(), null, { timeout: 5000 });
      const said = await page.evaluate((t) => {
        window.__say(t.split(' ').slice(0, 2).join(' '), false);
        const at = Date.now();
        window.__say(t, true);
        return at;
      }, hu);
      const shown = await page.waitForFunction((e) => (document.querySelector('[role="log"]')?.textContent.includes(e) ? Date.now() : false), en, { polling: 'raf', timeout: 8000 });
      textMs.push((await shown.jsonValue()) - said);
      const spokeAt = await page.waitForFunction((e) => window.__spoken.find((s) => s.text === e)?.t ?? false, en, { timeout: 8000 });
      speechMs.push((await spokeAt.jsonValue()) - said);
    }
    const spoken = await page.evaluate(() => window.__spoken.map((s) => [s.text, s.lang]));
    assert.deepEqual(spoken.map((s) => s[0]), PHRASES.map((p) => p[1]));
    assert.ok(spoken.every((s) => s[1] === 'en-US'));
  });

  await step('while a translation is spoken (no headphones), the recognizer is off; it comes back after', async () => {
    const log = await page.evaluate(() => window.__srLog.map((e) => e.ev));
    assert.ok(log.filter((e) => e === 'abort').length >= PHRASES.length, `aborts: ${log.join(',')}`);
    await page.waitForFunction(() => window.__srRunning(), null, { timeout: 5000 });
  });

  await step('a long run-on is translated in stable pieces before the speaker stops, without repeating words', async () => {
    const words = 'holnap reggel nyolc órakor találkozunk a pályaudvar előtt és utána elmegyünk együtt a nagy piacra'.split(' ');
    const before = await page.evaluate(() => document.querySelectorAll('[role="log"] li').length);
    for (let n = 2; n <= words.length; n++) {
      await page.evaluate((t) => window.__say(t, false), words.slice(0, n).join(' '));
      await page.waitForTimeout(260);
    }
    const mid = await page.evaluate(() => document.querySelectorAll('[role="log"] li').length);
    assert.ok(mid > before, 'an early piece appeared before the final');
    assert.equal(await page.evaluate(() => window.__srRunning()), true, 'still listening — speech waits for a pause (no headphones)');
    await page.evaluate((t) => window.__say(t, true), words.join(' '));
    await page.waitForTimeout(600);
    const originals = await page.evaluate(() => [...document.querySelectorAll('[role="log"] li [data-original]')].slice(-4).map((p) => p.textContent));
    const joined = originals.join(' ');
    for (const w of ['holnap', 'piacra']) assert.equal(joined.split(w).length - 1, 1, `"${w}" appears once: ${joined}`);
  });

  await step('the room speaking English instead → a hint offers to switch (never switches by itself)', async () => {
    for (const t of ['I think that the train is late again', 'and you know it is not the first time this week']) {
      await page.waitForFunction(() => window.__srRunning(), null, { timeout: 5000 });
      await page.evaluate((x) => window.__say(x, true), t);
      await page.waitForTimeout(250);
    }
    await page.getByText(/This sounds like English/).waitFor({ timeout: 5000 });
    assert.equal(await page.evaluate(() => window.__sr.at(-1).lang), 'hu-HU');
    await page.getByRole('button', { name: 'Dismiss' }).click();
  });

  await step('Speak translations off → text only; nothing new is spoken', async () => {
    await page.getByRole('button', { name: 'Speak translations' }).click();
    const n = await page.evaluate(() => window.__spoken.length);
    await page.waitForFunction(() => window.__srRunning(), null, { timeout: 5000 });
    await page.evaluate(() => window.__say('Jó napot kívánok', true));
    await page.waitForTimeout(500);
    // Duplicate within the window is dropped; say something new.
    await page.evaluate(() => window.__say('Mennyibe kerül a jegy', true));
    await page.waitForTimeout(800);
    assert.equal(await page.evaluate(() => window.__spoken.length), n);
    await page.getByRole('button', { name: 'Speak translations' }).click();
  });

  await step('Pause stops the recognizer at once; Resume starts listening again', async () => {
    await page.getByRole('button', { name: 'Pause listening' }).click();
    await page.waitForFunction(() => !window.__srRunning(), null, { timeout: 2000 });
    await page.getByText('Paused — not listening').waitFor();
    assert.doesNotMatch(await page.title(), /Listening/);
    await page.getByRole('button', { name: 'Resume listening' }).click();
    await page.waitForFunction(() => window.__srRunning(), null, { timeout: 3000 });
  });

  await step('settings: processing mode and where each step runs; On-device pauses listening (no on-device Hungarian)', async () => {
    await page.getByRole('button', { name: 'Live translate settings' }).click();
    const sheet = page.getByRole('dialog', { name: 'Live translate settings' });
    await sheet.waitFor();
    assert.match(await sheet.innerText(), /Speech recognition\s*Cloud/);
    const n0 = inferenceRequests.length;
    await page.getByRole('radio', { name: 'On-device' }).click();
    await page.waitForFunction(() => !window.__srRunning(), null, { timeout: 2000 });
    assert.match(await sheet.innerText(), /Speech recognition\s*Unavailable/);
    await page.keyboard.press('Escape');
    await page.getByText(/no longer allow this|can’t recognise this language/).first().waitFor();
    assert.equal(inferenceRequests.length, n0, 'no inference request after switching to On-device');
    await page.getByRole('button', { name: 'Live translate settings' }).click();
    await page.getByRole('radio', { name: 'Cloud' }).click();
    await page.keyboard.press('Escape');
    await page.getByRole('button', { name: /Resume listening|Start listening/ }).click();
    await page.waitForFunction(() => window.__srRunning(), null, { timeout: 3000 });
  });

  await step('leaving the page (hidden) pauses listening; it stays paused when visible again', async () => {
    await page.evaluate(() => {
      Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true });
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await page.waitForFunction(() => !window.__srRunning(), null, { timeout: 2000 });
    await page.evaluate(() => {
      Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true });
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await page.waitForTimeout(500);
    assert.equal(await page.evaluate(() => window.__srRunning()), false);
    await page.getByRole('button', { name: 'Resume listening' }).click();
    await page.waitForFunction(() => window.__srRunning(), null, { timeout: 3000 });
  });

  await step('Clear removes every line; nothing from the room is in browser storage', async () => {
    await page.getByRole('button', { name: 'Clear translations' }).click();
    assert.equal(await page.evaluate(() => document.querySelectorAll('[role="log"] li').length), 0);
    const stored = await page.evaluate(() => JSON.stringify({ ...localStorage }) + JSON.stringify({ ...sessionStorage }));
    for (const [hu, en] of PHRASES) {
      assert.ok(!stored.includes(hu) && !stored.includes(en), 'transcript in storage');
    }
  });

  await step('Back stops listening and speaking; the tab no longer says Listening', async () => {
    await page.getByRole('button', { name: 'Back' }).click();
    await page.waitForFunction(() => !window.__srRunning(), null, { timeout: 2000 });
    await page.getByRole('button', { name: 'Start a call' }).waitFor();
    assert.doesNotMatch(await page.title(), /Listening/);
  });

  await step('no page errors', async () => assert.deepEqual(errors, []));

  out.latency = {
    label: 'SIMULATED — scripted recognizer final → UI/speech; real server, stubbed provider, loopback network',
    final_to_text_ms: { n: textMs.length, p50: pct(textMs, 50), p95: pct(textMs, 95), max: Math.max(...textMs) },
    final_to_speech_start_ms: { n: speechMs.length, p50: pct(speechMs, 50), p95: pct(speechMs, 95), max: Math.max(...speechMs) },
    excludes: 'microphone capture, real recognizer latency (cloud ASR typically adds hundreds of ms to seconds), real provider latency, Bluetooth output latency',
  };
  console.log(`  latency (simulated): final→text p50 ${out.latency.final_to_text_ms.p50}ms p95 ${out.latency.final_to_text_ms.p95}ms · final→speech p50 ${out.latency.final_to_speech_start_ms.p50}ms p95 ${out.latency.final_to_speech_start_ms.p95}ms`);
  await context.close();
} catch {
  exitCode = 1;
} finally {
  await browser.close();
  await stack.stop();
}
out.steps = results;
mkdirSync(path.join(root, 'docs/phase8/results'), { recursive: true });
writeFileSync(path.join(root, 'docs/phase8/results/ambient-e2e.json'), JSON.stringify(out, null, 2) + '\n');
const failed = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - failed}/${results.length} ambient e2e steps passed`);
process.exit(exitCode || failed ? 1 : 0);
