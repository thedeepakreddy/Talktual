// Observes, in a real browser, whether speech recognition / the microphone are
// active in a connected call while nobody is holding the talk button.
// Usage: node e2e/privacy-probe.mjs [repoRoot]
import path from 'node:path';
import { loadChromium, startStack, pairPeers, root as defaultRoot } from './harness.mjs';

const cwd = path.resolve(process.argv[2] || defaultRoot);
const stack = await startStack({ apiPort: 8912, webPort: 4212, cwd, outDir: path.join(cwd, 'e2e/.dist-latency') });
const chromium = await loadChromium();
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });
let code = 0;
try {
  const { host } = await pairPeers(browser, stack.web);
  await host.page.waitForTimeout(3000); // idle in a connected call, no press, no consent given
  const r = await host.page.evaluate(() => ({
    recognizerRunning: window.__srRunning(),
    recognizerEvents: window.__srLog.map((e) => e.ev),
  }));
  console.log('idle connected call, 3s, button never pressed:', JSON.stringify(r));
  console.log('JSON ' + JSON.stringify(r));
} catch (err) {
  console.error(err);
  code = 1;
} finally {
  await browser.close();
  await stack.stop();
  process.exit(code);
}
