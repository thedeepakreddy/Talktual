// Phase 6 M6.0 evidence: what on-device runtimes this machine's Chromium actually exposes.
// Each probe runs in its own page (some APIs crash the renderer); results go to stdout as JSON.
// Usage: node e2e/probe-local-capabilities.mjs
import http from 'node:http';
import os from 'node:os';
import { loadChromium } from './harness.mjs';

const probes = {
  apis: `({ua:navigator.userAgent, secure:isSecureContext, webSpeech: !!window.webkitSpeechRecognition, srAvailableFn: !!(window.webkitSpeechRecognition||{}).available, srInstallFn: !!(window.webkitSpeechRecognition||{}).install, translatorApi: typeof Translator, languageDetectorApi: typeof LanguageDetector, webgpuApi: !!navigator.gpu, wasm: typeof WebAssembly === 'object', cores: navigator.hardwareConcurrency, deviceMemoryGB: navigator.deviceMemory, batteryApi: !!navigator.getBattery})`,
  speechCloudAvailable: `(async()=>window.webkitSpeechRecognition.available({langs:['en-US']}))()`,
  speechOnDeviceAvailable: `(async()=>window.webkitSpeechRecognition.available({langs:['en-US'],processLocally:true}))()`,
  translatorEnEs: `(async()=>Promise.race([Translator.availability({sourceLanguage:'en',targetLanguage:'es'}), new Promise(r=>setTimeout(()=>r('no-answer-3s'),3000))]))()`,
  languageDetector: `(async()=>Promise.race([LanguageDetector.availability(), new Promise(r=>setTimeout(()=>r('no-answer-3s'),3000))]))()`,
  voices: `(async()=>{ await new Promise(r=>setTimeout(r,1500)); const v=speechSynthesis.getVoices(); return {total:v.length, localService:v.filter(x=>x.localService).length} })()`,
  webgpuAdapter: `(async()=>{ const a = await navigator.gpu.requestAdapter(); return a ? {fallback:a.isFallbackAdapter} : null })()`,
};
const srv = http.createServer((_q, res) => { res.setHeader('content-type', 'text/html'); res.end('<p>probe</p>'); }).listen(4298);
const chromium = await loadChromium();
const b = await chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] });
const out = { at: new Date().toISOString(), host: { platform: process.platform, arch: process.arch, cpus: os.cpus().length, cpuModel: os.cpus()[0]?.model, memGB: Math.round(os.totalmem() / 2 ** 30) }, chromium: b.version(), results: {} };
for (const [k, code] of Object.entries(probes)) {
  const p = await b.newPage();
  let crashed = false;
  p.on('crash', () => (crashed = true));
  await p.goto('http://localhost:4298/');
  try {
    out.results[k] = await Promise.race([p.evaluate(code), new Promise((r) => setTimeout(() => r('no-answer-8s'), 8000))]);
  } catch (e) {
    out.results[k] = crashed ? 'RENDERER-CRASH' : `error: ${String(e.message).split('\n')[0]}`;
  }
  await p.close().catch(() => {});
}
await b.close();
srv.close();
console.log(JSON.stringify(out, null, 1));
