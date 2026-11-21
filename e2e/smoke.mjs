// End-to-end smoke test: two real browser contexts, the real production build,
// the real signaling/API server — with only the external edges faked:
//
//   * the translation provider (server-side fetch stub; no live API calls),
//   * the browser's speech recognizer (scripted phrases instead of a mic),
//   * speechSynthesis (records what would have been spoken).
//
// Usage: npm run test:e2e   (needs Playwright + Chromium; see TEST_REPORT.md)

import { spawn, spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import assert from 'node:assert/strict';

const require = createRequire(import.meta.url);
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const { createServer } = require(path.join(root, 'signaling-server/server.js'));

let chromium;
try {
  ({ chromium } = require('playwright'));
} catch {
  ({ chromium } = await import('playwright'));
}

const API_PORT = Number(process.env.E2E_API_PORT || 8899);
const WEB_PORT = Number(process.env.E2E_WEB_PORT || 4199);
const OUT_DIR = path.join(root, 'e2e/.dist');

// --- Fake translation provider (MyMemory wire format) ----------------------
const DICT = {
  'en|es': { 'hello there': 'hola ahí', 'how are you': 'cómo estás' },
  'es|en': { gracias: 'thanks' },
};
const providerCalls = [];
async function fakeFetch(url) {
  const u = new URL(String(url));
  const q = u.searchParams.get('q');
  const pair = u.searchParams.get('langpair');
  providerCalls.push({ pair, q });
  if (q === 'fail me') return new Response(JSON.stringify({ responseStatus: 500, responseDetails: 'boom' }), { status: 200 });
  const t = DICT[pair]?.[q];
  return new Response(
    JSON.stringify({ responseStatus: 200, responseData: { translatedText: t ?? `[${pair}] ${q}` } }),
    { status: 200 }
  );
}

// --- Browser fakes ------------------------------------------------------------
const INIT_SCRIPT = () => {
  window.__sr = [];
  class FakeSR {
    constructor() {
      window.__sr.push(this);
    }
    start() {}
    stop() {
      setTimeout(() => this.onend && this.onend(), 0);
    }
    abort() {}
  }
  window.webkitSpeechRecognition = FakeSR;
  window.SpeechRecognition = FakeSR;
  window.__say = (text, isFinal = true) => {
    const r = window.__sr[window.__sr.length - 1];
    const res = [{ transcript: text, confidence: 1 }];
    res.isFinal = isFinal;
    r.onresult({ resultIndex: 0, results: [res] });
  };
  window.__spoken = [];
  const synth = {
    speak(u) {
      if (u.text) window.__spoken.push({ text: u.text, lang: u.lang });
      setTimeout(() => u.onend && u.onend(), 30);
    },
    cancel() {},
    getVoices() {
      return [];
    },
    onvoiceschanged: null,
  };
  Object.defineProperty(window, 'speechSynthesis', { value: synth, configurable: true });
};

// --- Harness ------------------------------------------------------------------
const results = [];
async function step(name, fn) {
  const t0 = Date.now();
  try {
    await fn();
    results.push({ name, ok: true, ms: Date.now() - t0 });
    console.log(`  ✔ ${name} (${Date.now() - t0}ms)`);
  } catch (err) {
    results.push({ name, ok: false, ms: Date.now() - t0, err: String(err?.message || err) });
    console.log(`  ✖ ${name}\n      ${String(err?.message || err).split('\n').join('\n      ')}`);
    throw err;
  }
}

function waitForHttp(url, timeoutMs = 20000) {
  const end = Date.now() + timeoutMs;
  return (async () => {
    while (Date.now() < end) {
      try {
        const r = await fetch(url);
        if (r.ok) return;
      } catch {
        /* not up yet */
      }
      await new Promise((r) => setTimeout(r, 200));
    }
    throw new Error(`timeout waiting for ${url}`);
  })();
}

const api = createServer({
  fetch: fakeFetch,
  elevenLabsApiKey: undefined,
  geminiApiKey: undefined,
  deeplApiKey: undefined,
  allowedOrigins: [],
  log: () => {},
});
await new Promise((r) => api.server.listen(API_PORT, '127.0.0.1', r));

console.log('building frontend…');
const build = spawnSync('npx', ['vite', 'build', '--outDir', OUT_DIR, '--emptyOutDir', '--logLevel', 'warn'], {
  cwd: root,
  env: { ...process.env, VITE_SIGNALING_URL: `ws://127.0.0.1:${API_PORT}` },
  stdio: 'inherit',
});
if (build.status !== 0) throw new Error('vite build failed');

const preview = spawn('npx', ['vite', 'preview', '--outDir', OUT_DIR, '--port', String(WEB_PORT), '--strictPort', '--host', '127.0.0.1'], {
  cwd: root,
  stdio: 'ignore',
});
const WEB = `http://127.0.0.1:${WEB_PORT}`;

const browser = await chromium.launch({
  args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'],
});
const pageErrors = [];
let exitCode = 0;

async function newPeer(locale) {
  const context = await browser.newContext({ locale, permissions: ['microphone'] });
  await context.addInitScript(INIT_SCRIPT);
  const page = await context.newPage();
  page.on('pageerror', (e) => pageErrors.push(`${locale}: ${e.message}`));
  return { context, page };
}

async function talk(page, text) {
  const ptt = page.getByRole('button', { name: 'Push to talk' });
  await ptt.hover();
  await page.mouse.down();
  await page.evaluate((t) => window.__say(t), text);
  await page.mouse.up();
}

try {
  await waitForHttp(WEB);
  console.log(`Chromium ${browser.version()} — running scenarios`);

  const host = await newPeer('en-US');
  const guest = await newPeer('es-ES');
  let code = '';

  await step('host starts a channel and gets a 6-char code', async () => {
    await host.page.goto(WEB);
    await host.page.getByLabel('Your name').fill('Ana');
    await host.page.getByRole('button', { name: /start a channel/i }).click();
    await host.page.getByText('Invite someone').waitFor();
    await host.page.waitForFunction(() => document.querySelectorAll('span.tnum').length === 6 && !document.body.innerText.includes('······'));
    code = (await host.page.locator('span.tnum').allTextContents()).join('');
    assert.match(code, /^[A-Z0-9]{6}$/);
  });

  await step('guest opens the ?join= link and both land in the call', async () => {
    await guest.page.goto(`${WEB}/?join=${code}`);
    await guest.page.getByLabel('Your name').fill('Luis');
    await guest.page.getByRole('button', { name: /join a channel/i }).click();
    await guest.page.getByRole('button', { name: /^join$/i }).click();
    await Promise.all([
      host.page.getByText('Hold to talk').waitFor({ timeout: 15000 }),
      guest.page.getByText('Hold to talk').waitFor({ timeout: 15000 }),
    ]);
    // Language handshake: each side sees the other's language.
    await host.page.getByText('Español').waitFor({ timeout: 5000 });
    await guest.page.getByText('English').waitFor({ timeout: 5000 });
  });

  let transport = 'unknown';
  await step('transport is reported (P2P data channel or relay)', async () => {
    await host.page.waitForTimeout(1500);
    const footer = await host.page.locator('text=/^(Direct|Relayed)$/').first().textContent();
    transport = footer;
    assert.ok(['Direct', 'Relayed'].includes(footer));
  });

  await step('host speaks EN → guest sees and hears ES translation', async () => {
    await talk(host.page, 'hello there');
    await guest.page.getByText('hola ahí').waitFor({ timeout: 10000 });
    await guest.page.waitForFunction(() => window.__spoken.some((s) => s.text === 'hola ahí' && s.lang === 'es-ES'));
  });

  await step('guest speaks ES → host hears EN translation', async () => {
    await talk(guest.page, 'gracias');
    await host.page.getByText('thanks').waitFor({ timeout: 10000 });
    await host.page.waitForFunction(() => window.__spoken.some((s) => s.text === 'thanks' && s.lang === 'en-US'));
  });

  await step('multi-segment turn arrives in spoken order', async () => {
    const ptt = host.page.getByRole('button', { name: 'Push to talk' });
    await ptt.hover();
    await host.page.mouse.down();
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
    // Spoken with an English voice, since it's English text.
    await guest.page.waitForFunction(() => window.__spoken.some((s) => s.text === 'fail me' && s.lang === 'en-US'));
  });

  await step('mid-call language switch retargets the next turn without reconnecting', async () => {
    await guest.page.getByRole('button', { name: /you speak spanish/i }).click();
    await guest.page.getByLabel('Search languages').fill('French');
    await guest.page.getByRole('button', { name: /Français/ }).click();
    await host.page.getByText('Français').waitFor({ timeout: 5000 });
    await talk(host.page, 'hello there');
    await guest.page.waitForFunction(() => window.__spoken.some((s) => s.text === '[en|fr] hello there' && s.lang === 'fr-FR'));
  });

  await step('transcript lists both sides', async () => {
    await host.page.getByRole('button', { name: 'Show transcript' }).click();
    const dialog = host.page.getByRole('dialog', { name: 'Transcript' });
    await dialog.getByText('hello there').first().waitFor();
    await dialog.getByText('Luis').first().waitFor();
    await dialog.getByText('Not translated — translation was unavailable').waitFor();
    await dialog.getByRole('button', { name: 'Close transcript' }).click();
  });

  await step('host leaves → guest is told and returned home', async () => {
    await host.page.getByRole('button', { name: 'Leave channel' }).click();
    await host.page.getByRole('button', { name: /start a channel/i }).waitFor();
    await guest.page.getByText(/left the channel/i).first().waitFor({ timeout: 10000 });
    await guest.page.getByRole('button', { name: /start a channel/i }).waitFor({ timeout: 10000 });
  });

  await step('server cleaned up every session after the call', async () => {
    await new Promise((r) => setTimeout(r, 300));
    assert.equal(api.sessions.size, 0, `sessions left: ${[...api.sessions.keys()]}`);
  });

  await step('DEF-C04: joining an unknown code fails with a message (no infinite spinner)', async () => {
    await guest.page.goto(`${WEB}/?join=QQQQQQ`);
    await guest.page.getByLabel('Your name').fill('Luis');
    await guest.page.getByRole('button', { name: /join a channel/i }).click();
    await guest.page.getByRole('button', { name: /^join$/i }).click();
    await guest.page.getByRole('alert').getByText(/no one is waiting/i).waitFor({ timeout: 20000 });
    await new Promise((r) => setTimeout(r, 300));
    assert.equal(api.sessions.size, 0, 'abandoned lobby seat was released');
  });

  await step('no uncaught page errors in either browser', async () => {
    assert.deepEqual(pageErrors, []);
  });

  assert.ok(
    providerCalls.every((c) => typeof c.q === 'string'),
    'all provider traffic went to the stub'
  );
  console.log(`\nTransport observed: ${transport}. Provider calls (stubbed): ${providerCalls.length}.`);
} catch {
  exitCode = 1;
} finally {
  await browser.close();
  preview.kill();
  await api.close();
  const passed = results.filter((r) => r.ok).length;
  console.log(`\n${passed}/${results.length} e2e steps passed`);
  process.exit(exitCode);
}
