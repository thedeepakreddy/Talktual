// Shared E2E harness: builds the frontend against an in-process server whose
// provider calls are stubbed, serves it with `vite preview`, and opens Chromium
// contexts with scripted speech recognition and recorded speech synthesis.
// No live provider calls are made.

import { spawn, spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const require = createRequire(import.meta.url);
export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

export async function loadChromium() {
  try {
    return require('playwright').chromium;
  } catch {
    return (await import('playwright')).chromium;
  }
}

/** Fake translation (MyMemory wire format) and Scribe STT. */
export function makeFakeFetch({ dict = {}, providerCalls = [], sttText = 'hello there' } = {}) {
  return async (url, init) => {
    const u = new URL(String(url));
    if (u.hostname === 'api.elevenlabs.io' && u.pathname === '/v1/speech-to-text') {
      providerCalls.push({ kind: 'stt' });
      return new Response(JSON.stringify({ text: sttText, language_code: 'en' }), { status: 200 });
    }
    if (u.hostname === 'api.elevenlabs.io') {
      providerCalls.push({ kind: 'tts' });
      return new Response('tts disabled in e2e', { status: 500 });
    }
    const q = u.searchParams.get('q');
    const pair = u.searchParams.get('langpair');
    providerCalls.push({ kind: 'translate', pair, q });
    if (q === 'fail me') return new Response(JSON.stringify({ responseStatus: 500, responseDetails: 'boom' }), { status: 200 });
    const t = dict[pair]?.[q];
    return new Response(
      JSON.stringify({ responseStatus: 200, responseData: { translatedText: t ?? `[${pair}] ${q}` } }),
      { status: 200 }
    );
  };
}

/**
 * Browser fakes. `opts.noWebSpeech` removes the Web Speech API to exercise the
 * server-STT path. Speech synthesis records `{text, lang, t}` (t = Date.now()).
 */
export function initScript(opts = {}) {
  // Track this page's WebSockets so a test can cut *this* peer's connection.
  const NativeWS = window.WebSocket;
  window.__sockets = [];
  window.WebSocket = class extends NativeWS {
    constructor(...args) {
      super(...args);
      window.__sockets.push(this);
    }
  };
  window.__dropSockets = () => window.__sockets.forEach((s) => s.readyState <= 1 && s.close(4000, 'test drop'));
  window.__sr = [];
  window.__srLog = [];
  if (opts.noWebSpeech) {
    delete window.webkitSpeechRecognition;
    delete window.SpeechRecognition;
  } else {
    class FakeSR {
      constructor() {
        window.__sr.push(this);
        this.running = false;
      }
      start() {
        if (this.running) throw new DOMException('already started', 'InvalidStateError');
        this.running = true;
        window.__srLog.push({ ev: 'start', t: Date.now() });
      }
      stop() {
        if (!this.running) return;
        this.running = false;
        window.__srLog.push({ ev: 'stop', t: Date.now() });
        setTimeout(() => this.onend && this.onend(), 0);
      }
      abort() {
        if (!this.running) return;
        this.running = false;
        window.__srLog.push({ ev: 'abort', t: Date.now() });
        setTimeout(() => this.onend && this.onend(), 0);
      }
    }
    window.webkitSpeechRecognition = FakeSR;
    window.SpeechRecognition = FakeSR;
  }
  window.__srRunning = () => window.__sr.some((r) => r.running);
  window.__say = (text, isFinal = true) => {
    const r = [...window.__sr].reverse().find((x) => x.running) || window.__sr[window.__sr.length - 1];
    if (!r) throw new Error('no recognizer');
    const res = [{ transcript: text, confidence: 1 }];
    res.isFinal = isFinal;
    r.onresult({ resultIndex: 0, results: [res] });
  };
  window.__spoken = [];
  const synth = {
    speak(u) {
      if (u.text) window.__spoken.push({ text: u.text, lang: u.lang, t: Date.now(), pitch: u.pitch, rate: u.rate });
      setTimeout(() => u.onend && u.onend(), 30);
    },
    cancel() {},
    getVoices() {
      // opts.simulatedLocalVoices: SIMULATED on-device voices (the test hardware has none).
      return (opts.simulatedLocalVoices || []).map((lang) => ({ name: `Simulated ${lang}`, lang, localService: true, voiceURI: `sim-${lang}`, default: false }));
    },
    onvoiceschanged: null,
  };
  Object.defineProperty(window, 'speechSynthesis', { value: synth, configurable: true });
  if (opts.simulatedLocalVoices) {
    // SIMULATION: synthesis is faked anyway; a fake utterance accepts the fake voices.
    window.SpeechSynthesisUtterance = class {
      constructor(text) {
        this.text = text;
        this.lang = '';
        this.voice = null;
      }
    };
  }
}

export async function startStack({ apiPort, webPort, serverOverrides = {}, fetchOpts = {}, outDir, cwd = root, buildEnv = {} }) {
  const { createServer } = require(path.join(cwd, 'signaling-server/server.js'));
  const providerCalls = [];
  const api = createServer({
    fetch: makeFakeFetch({ ...fetchOpts, providerCalls }),
    elevenLabsApiKey: undefined,
    geminiApiKey: undefined,
    deeplApiKey: undefined,
    allowedOrigins: [],
    log: () => {},
    ...serverOverrides,
  });
  await new Promise((r) => api.server.listen(apiPort, '127.0.0.1', r));

  const dist = outDir || path.join(cwd, 'e2e/.dist');
  const build = spawnSync('npx', ['vite', 'build', '--outDir', dist, '--emptyOutDir', '--logLevel', 'error'], {
    cwd,
    env: { ...process.env, VITE_SIGNALING_URL: `ws://127.0.0.1:${apiPort}`, ...buildEnv },
    stdio: 'inherit',
  });
  if (build.status !== 0) throw new Error('vite build failed');
  const preview = spawn(
    'npx',
    ['vite', 'preview', '--outDir', dist, '--port', String(webPort), '--strictPort', '--host', '127.0.0.1'],
    { cwd, stdio: 'ignore' }
  );
  const web = `http://127.0.0.1:${webPort}`;
  const end = Date.now() + 20000;
  for (;;) {
    try {
      if ((await fetch(web)).ok) break;
    } catch {
      /* starting */
    }
    if (Date.now() > end) throw new Error('preview did not start');
    await new Promise((r) => setTimeout(r, 200));
  }
  return {
    api,
    web,
    providerCalls,
    async stop() {
      preview.kill();
      await api.close();
    },
  };
}

/** Accepts the speech-processing disclosure if this build shows one. */
export async function acceptConsentIfShown(page) {
  const btn = page.getByRole('button', { name: /^allow microphone/i });
  if (await btn.count()) await btn.first().click();
}

export async function talk(page, text) {
  const ptt = page.getByRole('button', { name: 'Push to talk' });
  await ptt.hover();
  const pressedAt = await page.evaluate(() => Date.now());
  await page.mouse.down();
  await page.waitForFunction(() => window.__srRunning ? window.__srRunning() || window.__sr.length > 0 : true);
  const saidAt = await page.evaluate((t) => {
    const at = Date.now();
    window.__say(t);
    return at;
  }, text);
  const releasedAt = await page.evaluate(() => Date.now());
  await page.mouse.up();
  return { pressedAt, saidAt, releasedAt };
}

/** Host starts a channel, guest joins via link; returns once both are in the call. */
export async function pairPeers(browser, web, initOpts = {}) {
  const mk = async (locale, opts) => {
    const context = await browser.newContext({ locale, permissions: ['microphone'] });
    await context.addInitScript(initScript, opts);
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', (e) => errors.push(e.message));
    return { context, page, errors };
  };
  const host = await mk('en-US', initOpts.host || {});
  const guest = await mk('es-ES', initOpts.guest || {});
  await host.page.goto(initOpts.debug ? `${web}/?debug=1` : web);
  await host.page.getByLabel('Your name').fill('Ana');
  await host.page.getByRole('button', { name: /start a channel/i }).click();
  await host.page.waitForFunction(
    () => document.querySelectorAll('span.tnum').length === 6 && !document.body.innerText.includes('······')
  );
  const code = (await host.page.locator('span.tnum').allTextContents()).join('');
  await guest.page.goto(`${web}/?join=${code}`);
  await guest.page.getByLabel('Your name').fill('Luis');
  await guest.page.getByRole('button', { name: /join a channel/i }).click();
  await guest.page.getByRole('button', { name: /^join$/i }).click();
  await Promise.all([
    host.page.getByRole('button', { name: 'Push to talk' }).waitFor({ timeout: 15000 }),
    guest.page.getByRole('button', { name: 'Push to talk' }).waitFor({ timeout: 15000 }),
  ]);
  await host.page.getByText('Español').first().waitFor({ timeout: 5000 });
  return { host, guest, code };
}
