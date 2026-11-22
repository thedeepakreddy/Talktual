import { describe, it, expect, vi, afterEach } from 'vitest';
import { Phrasebook, PhrasebookRuntime } from './phrasebook';
import { RuntimeRegistry, validLocalOutput, DEFAULT_REGISTRY_LIMITS } from './registry';
import { BrowserTranslatorRuntime, LocalVoiceRuntime, OnDeviceAsrRuntime } from './platform';
import type { LocalTranslateResult, RuntimeInfo, TranslateRuntime } from './types';
import { ProcessingPolicy, PolicyBlockedError } from '../privacy/policy';
import { NetworkMonitor } from '../privacy/network';

const clock = { now: () => Date.now(), setTimeout: (f: () => void, ms: number) => setTimeout(f, ms), clearTimeout: (t: unknown) => clearTimeout(t as number) };
afterEach(() => vi.useRealTimers());

describe('phrasebook (on-device, en↔es)', () => {
  const pb = new Phrasebook();
  pb.load();
  const tr = (t: string, from: 'en' | 'es') => pb.translate(t, from, from === 'en' ? 'es' : 'en')?.text ?? null;

  it.each([
    ['where is the train station', 'en', '¿Dónde está la estación de tren?'],
    ['um where is the bathroom please', 'en', '¿Dónde está el baño, por favor?'],
    ['two tickets please', 'en', 'Dos billetes, por favor'],
    ['a table for four', 'en', 'Una mesa para cuatro'],
    ['at five thirty', 'en', 'A las cinco y media'],
    ['how do i get to the airport', 'en', '¿Cómo llego al aeropuerto?'],
    ['is there a pharmacy near here', 'en', '¿Hay una farmacia cerca?'],
    ['excuse me where is the exit', 'en', 'Disculpe, ¿dónde está la salida?'],
    ['hello where is the hotel', 'en', 'Hola, ¿dónde está el hotel?'],
    ['it costs 20 euros', 'en', 'Cuesta 20 euros'],
    ['¿Dónde está la farmacia?', 'es', 'Where is the pharmacy?'],
    ['cómo llego al aeropuerto', 'es', 'How do I get to the airport?'],
    ['necesito un médico', 'es', 'I need a doctor'],
    ['a la una', 'es', 'At one'],
    ['no entiendo', 'es', "I don't understand"],
  ])('%s → %s', (input, from, want) => {
    expect(tr(input, from as 'en')).toBe(want);
  });

  it('names are copied verbatim (incl. protection tokens)', () => {
    expect(tr('my name is Rose', 'en')).toBe('Me llamo Rose');
    expect(tr('me llamo QX0Q', 'es')).toBe('My name is QX0Q');
  });

  it('never translates partially: uncovered words → not covered (null)', () => {
    expect(tr('I think the government should invest more in trains', 'en')).toBeNull();
    expect(tr('where is the spaceport', 'en')).toBeNull(); // unknown slot value
    expect(tr('hello and where is the dragon', 'en')).toBeNull();
    expect(tr('two tickets', 'fr' as 'en')).toBeNull();
  });

  it('runtime adapter: info is honest, load/unload lifecycle, unsupported pair', async () => {
    const rt = new PhrasebookRuntime();
    expect(rt.info()).toMatchObject({ task: 'translate', languages: ['en>es', 'es>en'], verification: 'verified-test-hardware' });
    expect(rt.supports('en-US', 'fr-FR')).toBe(false);
    await rt.load();
    expect(rt.info().resources.loadMs).not.toBeNull();
    expect(await rt.translate({ text: 'thank you', from: 'en-US', to: 'es-ES' })).toEqual({ text: 'Gracias', coverage: 'exact' });
    rt.unload();
    expect(await rt.translate({ text: 'thank you', from: 'en-US', to: 'fr-FR' })).toBeNull();
  });
});

class FakeRt implements TranslateRuntime {
  loads = 0;
  unloads = 0;
  constructor(
    readonly id: string,
    private readonly impl: (t: string) => Promise<LocalTranslateResult>,
    private readonly ready = 'ready' as const
  ) {}
  info(): RuntimeInfo {
    return { v: 1, id: this.id, task: 'translate', languages: ['en>es'], model: { name: 'x', version: '1', license: 'x', sizeKB: 1, install: 'x' }, platform: 'x', readiness: this.ready, constraints: [], quality: 'x', resources: { memoryKB: null, loadMs: null }, verification: 'unverified' };
  }
  async discover() {
    return this.ready;
  }
  supports() {
    return true;
  }
  async load() {
    this.loads++;
  }
  unload() {
    this.unloads++;
  }
  translate(r: { text: string }) {
    return this.impl(r.text);
  }
  dispose() {}
}

describe('runtime registry (bounds, validation, lifecycle)', () => {
  it('validates output: empty, control chars, absurd length → rejected', () => {
    expect(validLocalOutput('hi', 'hola', 100)).toBe(true);
    expect(validLocalOutput('hi', '   ', 100)).toBe(false);
    expect(validLocalOutput('hi', 'a\u0007b', 100)).toBe(false);
    expect(validLocalOutput('hi', 'x'.repeat(200), 1000)).toBe(false);
  });

  it('bounded concurrency + queue: excess calls are refused (caller falls back), not queued forever', async () => {
    const release: Array<() => void> = [];
    const rt = new FakeRt('slow', () => new Promise((r) => release.push(() => r({ text: 'ok', coverage: 'exact' }))));
    const reg = new RuntimeRegistry(clock, { ...DEFAULT_REGISTRY_LIMITS, maxQueue: 2, callTimeoutMs: 10_000 });
    reg.addTranslator(rt);
    await reg.discover();
    const calls = Array.from({ length: 5 }, () => reg.translate({ text: 'x', from: 'en', to: 'es' }));
    await new Promise((r) => setTimeout(r, 0));
    const refused = await Promise.race([calls[3], new Promise((r) => setTimeout(() => r('pending'), 20))]);
    expect(refused).toEqual({ ok: false, reason: 'busy' });
    while (release.length) release.shift()!(), await new Promise((r) => setTimeout(r, 0));
    const res = await Promise.all(calls);
    expect(res.filter((r) => r.ok)).toHaveLength(3);
    expect(reg.stats.refusedBusy).toBe(2);
  });

  it('per-call timeout; invalid output rejected; cancellation honoured', async () => {
    vi.useFakeTimers();
    const reg = new RuntimeRegistry(clock, { ...DEFAULT_REGISTRY_LIMITS, callTimeoutMs: 100 });
    reg.addTranslator(new FakeRt('hang', () => new Promise(() => {})));
    await vi.advanceTimersByTimeAsync(0);
    const disc = reg.discover();
    await vi.advanceTimersByTimeAsync(0);
    await disc;
    const p = reg.translate({ text: 'x', from: 'en', to: 'es' });
    await vi.advanceTimersByTimeAsync(150);
    expect(await p).toEqual({ ok: false, reason: 'timeout' });
    const reg2 = new RuntimeRegistry(clock);
    reg2.addTranslator(new FakeRt('bad', async () => ({ text: '\u0000', coverage: 'exact' })));
    const d2 = reg2.discover();
    await vi.advanceTimersByTimeAsync(0);
    await d2;
    expect(await reg2.translate({ text: 'x', from: 'en', to: 'es' })).toEqual({ ok: false, reason: 'invalid-output' });
    const c = new AbortController();
    c.abort();
    expect(await reg2.translate({ text: 'x', from: 'en', to: 'es', signal: c.signal })).toEqual({ ok: false, reason: 'cancelled' });
  });

  it('loaded-runtime bound (LRU), idle unload, memory-pressure unloadAll, dispose', async () => {
    vi.useFakeTimers();
    const rts = ['a', 'b', 'c'].map((id) => new FakeRt(id, async () => null));
    const reg = new RuntimeRegistry(clock, { ...DEFAULT_REGISTRY_LIMITS, maxLoaded: 2, idleUnloadMs: 1000 });
    rts.forEach((r) => reg.addTranslator(r));
    const d = reg.discover();
    await vi.advanceTimersByTimeAsync(0);
    await d;
    await reg.translate({ text: 'x', from: 'en', to: 'es' }); // all three miss → all loaded in turn
    expect(reg.loadedCount).toBeLessThanOrEqual(2);
    await vi.advanceTimersByTimeAsync(1001);
    expect(reg.loadedCount).toBe(0);
    await reg.translate({ text: 'x', from: 'en', to: 'es' });
    reg.unloadAll();
    expect(reg.loadedCount).toBe(0);
    reg.dispose();
    expect(await reg.translate({ text: 'x', from: 'en', to: 'es' })).toEqual({ ok: false, reason: 'no-runtime' });
  });

  it('a hanging platform discovery is time-bounded → unknown, never blocks', async () => {
    vi.useFakeTimers();
    const reg = new RuntimeRegistry(clock, { ...DEFAULT_REGISTRY_LIMITS, discoverTimeoutMs: 200 });
    reg.addTranslator(new BrowserTranslatorRuntime({ availability: () => new Promise(() => {}), create: async () => ({ translate: async () => '' }) }, [['en', 'es']]));
    const p = reg.discover();
    await vi.advanceTimersByTimeAsync(250);
    const infos = await p;
    expect(infos[0].readiness).toBe('unknown');
    expect(reg.canTranslate('en', 'es')).toBe(false);
  });
});

describe('platform runtimes (gated on what the platform reports)', () => {
  it('Translator API: only "available" pairs are used; "downloadable" never triggers create()', async () => {
    const create = vi.fn(async () => ({ translate: async (t: string) => `ES:${t}`, destroy: vi.fn() }));
    const api = { availability: async (o: { sourceLanguage: string }) => (o.sourceLanguage === 'en' ? 'available' : 'downloadable'), create };
    const rt = new BrowserTranslatorRuntime(api, [['en', 'es'], ['es', 'en']]);
    expect(await rt.discover()).toBe('ready');
    expect(rt.supports('es', 'en')).toBe(false);
    expect(await rt.translate({ text: 'hola', from: 'es', to: 'en' })).toBeNull();
    expect(create).not.toHaveBeenCalled();
    expect(await rt.translate({ text: 'hello', from: 'en', to: 'es' })).toEqual({ text: 'ES:hello', coverage: 'exact' });
    expect(rt.info().verification).toBe('platform-reported');
    expect(await new BrowserTranslatorRuntime(undefined, [['en', 'es']]).discover()).toBe('unavailable');
    const dl = new BrowserTranslatorRuntime({ availability: async () => 'downloadable', create }, [['en', 'es']]);
    expect(await dl.discover()).toBe('downloadable');
  });

  it('voices: only localService voices count', async () => {
    const rt = new LocalVoiceRuntime(() => [{ lang: 'en-US', localService: false }, { lang: 'es-ES', localService: true }]);
    expect(await rt.discover()).toBe('ready');
    expect(rt.hasLocalVoice('es-MX')).toBe(true);
    expect(rt.hasLocalVoice('en-US')).toBe(false);
  });

  it('on-device ASR is disabled unless explicitly enabled (crash evidence on test hardware)', async () => {
    const available = vi.fn(async () => 'available');
    const off = new OnDeviceAsrRuntime({ available }, false, ['en-US']);
    expect(await off.discover()).toBe('disabled');
    expect(available).not.toHaveBeenCalled();
    const on = new OnDeviceAsrRuntime({ available }, true, ['en-US']);
    expect(await on.discover()).toBe('ready');
    expect(on.supports('en-GB')).toBe(true);
  });
});

describe('processing policy (central)', () => {
  const ctx = (d: boolean, c = true) => ({ deviceAvailable: d, cloudAvailable: c });
  it('mode semantics', () => {
    const p = new ProcessingPolicy('cloud', () => 0);
    expect(p.plan('translate', ctx(true)).steps).toEqual(['cloud', 'device']);
    expect(p.plan('translate', ctx(false)).steps).toEqual(['cloud']);
    p.setMode('hybrid');
    expect(p.plan('translate', ctx(true)).steps).toEqual(['device', 'cloud']);
    expect(p.plan('translate', ctx(false))).toMatchObject({ steps: ['cloud'], reason: 'cloud-fallback' });
    p.setMode('local-only');
    expect(p.plan('translate', ctx(true))).toMatchObject({ steps: ['device'], reason: 'local-only' });
    expect(p.plan('translate', ctx(false))).toMatchObject({ steps: [], reason: 'local-only' }); // fail closed
  });

  it('local-only NEVER yields a cloud step, for every task and context', () => {
    const p = new ProcessingPolicy('local-only', () => 0);
    for (const task of ['asr', 'translate', 'caption', 'tts'] as const) {
      for (const d of [true, false]) for (const c of [true, false]) for (const consent of [true, false]) {
        expect(p.plan(task, { deviceAvailable: d, cloudAvailable: c, cloudConsent: consent }).steps).not.toContain('cloud');
      }
      expect(() => p.beginRemote(task)).toThrow(PolicyBlockedError);
    }
    expect(p.counts.cloud).toBe(0);
  });

  it('offline: only device; peer request: only device; ASR without consent: no cloud', () => {
    const p = new ProcessingPolicy('cloud', () => 0);
    p.setNetwork('offline');
    expect(p.plan('translate', ctx(true))).toMatchObject({ steps: ['device'], reason: 'offline' });
    expect(p.plan('tts', ctx(false))).toMatchObject({ steps: [], reason: 'offline' });
    p.setNetwork('online');
    expect(p.plan('tts', { ...ctx(true), peerRequestedDevice: true }).steps).toEqual(['device']);
    expect(p.plan('asr', { ...ctx(false), cloudConsent: false }).steps).toEqual([]);
    p.setNetwork('providers-unreachable');
    expect(p.plan('translate', ctx(true)).steps).toEqual(['device']);
  });

  it('revocation: switching to local-only (or going offline) aborts remote work in flight', () => {
    const p = new ProcessingPolicy('hybrid', () => 0);
    const a = p.beginRemote('translate');
    const b = p.beginRemote('caption');
    expect(p.remoteInFlight).toBe(2);
    p.setMode('local-only');
    expect(a.signal.aborted && b.signal.aborted).toBe(true);
    expect(p.remoteInFlight).toBe(0);
    p.setMode('cloud');
    const c = p.beginRemote('tts');
    p.setNetwork('offline');
    expect(c.signal.aborted).toBe(true);
  });

  it('audit is enum-only and bounded', () => {
    const p = new ProcessingPolicy('cloud', () => 5, 3);
    for (let i = 0; i < 10; i++) p.plan('translate', ctx(true));
    expect(p.auditLog()).toHaveLength(3);
    expect(Object.keys(p.auditLog()[0]).sort()).toEqual(['decision', 'location', 'reason', 't', 'task']);
    p.clearAudit();
    expect(p.auditLog()).toEqual([]);
  });

  it('unknown mode → cloud; preview does not audit', () => {
    const p = new ProcessingPolicy('bogus' as never, () => 0);
    expect(p.mode).toBe('cloud');
    p.preview('asr', ctx(false));
    expect(p.auditLog()).toEqual([]);
  });
});

describe('network monitor', () => {
  it('browser offline, provider failures with cooldown, recovery', () => {
    let online = true;
    let t = 0;
    const subs: Array<() => void> = [];
    const states: string[] = [];
    const m = new NetworkMonitor({ onLine: () => online, subscribe: (fn) => (subs.push(fn), () => {}) }, () => t, (s) => states.push(s), 2, 1000);
    expect(m.state).toBe('online');
    m.networkFailure();
    expect(m.state).toBe('online');
    m.networkFailure();
    expect(m.state).toBe('providers-unreachable');
    t += 1001;
    m.refresh();
    expect(m.state).toBe('online');
    online = false;
    subs.forEach((f) => f());
    expect(states.at(-1)).toBe('offline');
    online = true;
    m.success();
    expect(m.state).toBe('online');
  });
});
