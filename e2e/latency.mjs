// Emulated end-to-end turn latency: PTT release on the speaker → speech
// synthesis starts on the listener, across two Chromium contexts, through the
// real server and data channel, with the translation provider stubbed (0 ms).
//
// What this measures: client pipeline + signaling/data channel + server
// overhead. What it does NOT measure: real speech-recognizer finalization,
// real provider latency, real TTS time-to-first-audio, real networks.
//
// Usage: node e2e/latency.mjs [repoRoot] [turns]

import path from 'node:path';
import { loadChromium, startStack, pairPeers, acceptConsentIfShown, talk, root as defaultRoot } from './harness.mjs';

const cwd = path.resolve(process.argv[2] || defaultRoot);
const TURNS = Number(process.argv[3] || 30);
const pct = (a, p) => [...a].sort((x, y) => x - y)[Math.min(a.length - 1, Math.floor((p / 100) * a.length))];

const stack = await startStack({ apiPort: 8911, webPort: 4211, cwd, outDir: path.join(cwd, 'e2e/.dist-latency') });
const chromium = await loadChromium();
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });
let code = 0;
try {
  const { host, guest } = await pairPeers(browser, stack.web);
  await acceptConsentIfShown(host.page);
  await acceptConsentIfShown(guest.page);
  await host.page.waitForTimeout(1500); // let P2P settle
  const transport = await host.page.locator('text=/^(Direct|Relayed)$/').first().textContent();
  const lat = [];
  const fromFinal = [];
  for (let i = 0; i < TURNS; i++) {
    const phrase = `turn number ${i}`;
    const expected = `[en|es] ${phrase}`;
    const { saidAt, releasedAt } = await talk(host.page, phrase);
    const handle = await guest.page.waitForFunction(
      (e) => window.__spoken.find((s) => s.text === e)?.t,
      expected,
      { timeout: 10000 }
    );
    const spokeAt = await handle.jsonValue();
    lat.push(spokeAt - releasedAt);
    fromFinal.push(spokeAt - saidAt);
    await host.page.waitForTimeout(150);
  }
  console.log(`repo: ${cwd}`);
  console.log(`transport: ${transport}; turns: ${lat.length}`);
  console.log(
    `release → listener speech start: p50 ${pct(lat, 50)}ms  p95 ${pct(lat, 95)}ms  max ${Math.max(...lat)}ms  min ${Math.min(...lat)}ms`
  );
  console.log(
    `final phrase → listener speech start: p50 ${pct(fromFinal, 50)}ms  p95 ${pct(fromFinal, 95)}ms  max ${Math.max(...fromFinal)}ms`
  );
  console.log(
    'JSON ' +
      JSON.stringify({
        transport,
        n: lat.length,
        release: { p50: pct(lat, 50), p95: pct(lat, 95), max: Math.max(...lat) },
        final: { p50: pct(fromFinal, 50), p95: pct(fromFinal, 95), max: Math.max(...fromFinal) },
      })
  );
} catch (err) {
  console.error(err);
  code = 1;
} finally {
  await browser.close();
  await stack.stop();
  process.exit(code);
}
