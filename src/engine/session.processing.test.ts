// Phase 6 processing modes, central policy enforcement, local runtimes, offline.
import { describe, it, expect, vi, afterEach } from 'vitest';
import { makeSession, connected, settle, memoryStorage } from '../test/engineFakes';
import type { CallMessage } from '../lib/webrtc';
import type { TranslationRequest, TranslationResult } from './ports';
import type { ProcessingMode } from '../privacy/policy';
import { PROCESSING_STORAGE_KEY } from './CommunicationSession';

type Seg = Extract<CallMessage, { t: 'segment' }>;

function net(initial = true) {
  let online = initial;
  const subs = new Set<() => void>();
  return {
    source: { onLine: () => online, subscribe: (fn: () => void) => (subs.add(fn), () => subs.delete(fn)) },
    set(v: boolean) {
      online = v;
      subs.forEach((f) => f());
    },
  };
}

async function setup(o: { mode?: ProcessingMode; translate?: (r: TranslationRequest) => Promise<TranslationResult>; liveCaptions?: boolean; peerCaps?: string[]; storage?: ReturnType<typeof memoryStorage>; network?: ReturnType<typeof net>; extra?: Record<string, unknown> } = {}) {
  const n = o.network ?? net();
  const clearCaches = vi.fn();
  const h = makeSession({
    consent: true,
    server: { liveCaptions: o.liveCaptions ?? false },
    translate: o.translate ?? (async (r) => ({ segmentId: r.segmentId, status: 'translated', text: `CLOUD(${r.text})`, attempts: 1, latencyMs: 1 })),
    deps: { processing: { mode: o.mode, network: n.source }, clearCaches, ...(o.storage ? { storage: o.storage } : {}), ...o.extra },
  });
  await connected(h);
  h.call.receive({ t: 'hello', lang: 'es-ES', name: 'Bea', caps: o.peerCaps ?? ['priv1'] });
  await settle();
  return { ...h, net: n, clearCaches };
}
const segs = (h: { call: { segments: () => Seg[] } }) => h.call.segments();
const peerSeg = (over: Partial<Seg>): Seg => ({ t: 'segment', id: 'p1', original: 'hola', translated: 'hola', sourceLang: 'es-ES', targetLang: 'en-US', sentAt: 1, ...over });

afterEach(() => vi.useRealTimers());

describe('rollback / negotiation', () => {
  it('off when the dependency is absent (Phase 5 behaviour); priv1 advertised when on', async () => {
    const off = makeSession({ consent: true });
    await connected(off);
    expect(off.session.getSnapshot().processing.available).toBe(false);
    expect(off.call.sent.find((m) => m.t === 'hello')).not.toHaveProperty('caps');
    const on = await setup();
    expect(on.call.sent.find((m) => m.t === 'hello')).toMatchObject({ caps: ['priv1'] });
    expect(on.session.getSnapshot().processing).toMatchObject({ available: true, mode: 'cloud', network: 'online' });
  });
});

describe('local-only: remote inference is blocked (fail closed)', () => {
  it('no speech recognition without an on-device recognizer; typing works; nothing goes to the cloud', async () => {
    const h = await setup({ mode: 'local-only' });
    h.session.pressTalk();
    await settle();
    expect(h.asr()).toBeUndefined(); // no recognizer constructed: no audio leaves the device
    expect(h.session.getSnapshot().notice?.message).toMatch(/type your message/i);
    h.session.sendText('where is the train station');
    h.session.sendText('I think the new tax policy is unfair to students');
    await settle(40);
    expect(h.translate).not.toHaveBeenCalled();
    const [covered, uncovered] = segs(h);
    expect(covered).toMatchObject({ original: 'where is the train station', translated: '¿Dónde está la estación de tren?', priv: 'device' });
    expect(covered).not.toHaveProperty('untranslated');
    expect(uncovered).toMatchObject({ translated: 'I think the new tax policy is unfair to students', untranslated: true, priv: 'device' });
    const rec = h.session.getSnapshot().segments.filter((s) => s.speaker === 'local');
    expect(rec.map((r) => [r.processedOn, r.typed])).toEqual([['device', true], [undefined, true]]);
    const speech = h.session.getSnapshot().processing.capabilities.find((c) => c.task === 'speech-input')!;
    expect(speech.state).toBe('unavailable');
    expect(h.session.getSnapshot().processing.degraded).toBe(true);
  });

  it('incoming speech: no remote fix-up translation, no remote captions, on-device voice only', async () => {
    const translateCaption = vi.fn(async (t: string) => `CAP(${t})`);
    const h = await setup({ mode: 'local-only', liveCaptions: true, extra: { translateCaption } });
    h.call.receive({ t: 'partial', text: 'dónde está', lang: 'es-ES' });
    await settle();
    await new Promise((r) => setTimeout(r, 450));
    h.call.receive(peerSeg({ id: 'x1', original: 'necesito un médico', translated: 'necesito un médico', targetLang: 'es-ES' })); // needs a fix-up
    h.call.receive(peerSeg({ id: 'x2', original: 'el museo nuevo es precioso', translated: 'el museo nuevo es precioso', targetLang: 'es-ES' }));
    await settle(40);
    expect(h.translate).not.toHaveBeenCalled();
    expect(translateCaption).not.toHaveBeenCalled();
    const items = [...h.playback.played, ...h.playback.queue];
    expect(items.map((i) => i.text)).toEqual(['I need a doctor', 'el museo nuevo es precioso']);
    expect(items.every((i) => i.ttsRoute?.() === 'device-only')).toBe(true);
  });

  it('switching to local-only mid-flight aborts the remote translation; it is NOT resumed; on-device takes over or the words go untranslated', async () => {
    const signals: AbortSignal[] = [];
    const h = await setup({
      mode: 'cloud',
      translate: (r) =>
        new Promise((resolve) => {
          signals.push(r.signal!);
          r.signal!.addEventListener('abort', () => resolve({ segmentId: r.segmentId, status: 'cancelled', text: r.text, attempts: 1, latencyMs: 0 }));
        }),
    });
    h.session.sendText('thank you');
    h.session.sendText('the meeting moved to the old library');
    await settle();
    expect(signals).toHaveLength(1);
    h.session.setProcessingMode('local-only');
    await settle(40);
    expect(signals[0].aborted).toBe(true);
    expect(h.translate).toHaveBeenCalledTimes(1); // never re-sent to the cloud
    expect(segs(h).map((s) => [s.translated, s.untranslated ?? false])).toEqual([
      ['Gracias', false],
      ['the meeting moved to the old library', true],
    ]);
    expect(h.clearCaches).toHaveBeenCalled();
  });

  it('cloud speech recognition in progress is stopped when switching to local-only', async () => {
    const h = await setup({ mode: 'cloud' });
    h.session.pressTalk();
    await settle();
    const asr = h.asr()!;
    expect(asr.active).toBe(true);
    h.session.setProcessingMode('local-only');
    await settle();
    expect(asr.aborts).toBe(1);
    expect(h.session.getSnapshot().capturing).toBe(false);
  });
});

describe('hybrid and cloud routing', () => {
  it('hybrid: covered phrase on-device (no cloud call); uncovered → cloud', async () => {
    const h = await setup({ mode: 'hybrid' });
    h.session.sendText('thank you very much');
    h.session.sendText('the conference starts after lunch');
    await settle(40);
    expect(h.translate).toHaveBeenCalledTimes(1);
    expect(segs(h).map((s) => s.translated)).toEqual(['Gracias', 'CLOUD(the conference starts after lunch)']);
    expect(h.session.getSnapshot().segments.filter((s) => s.speaker === 'local').map((s) => s.processedOn)).toEqual(['device', 'cloud']);
  });

  it('cloud: cloud first; network failures → on-device fallback, then providers marked unreachable (no retry loop)', async () => {
    const fail = async (r: TranslationRequest): Promise<TranslationResult> => ({ segmentId: r.segmentId, status: 'failed', text: r.text, error: { code: 'network', message: 'x' }, attempts: 2, latencyMs: 1 });
    const h = await setup({ mode: 'cloud', translate: fail });
    h.session.sendText('thank you');
    h.session.sendText('good morning');
    await settle(40);
    expect(segs(h).map((s) => s.translated)).toEqual(['Gracias', 'Buenos días']);
    expect(h.session.getSnapshot().processing.network).toBe('providers-unreachable');
    h.session.sendText('hello');
    await settle(40);
    expect(h.translate).toHaveBeenCalledTimes(2); // third phrase did not try the unreachable cloud again
  });
});

describe('offline', () => {
  it('browser offline → only on-device; back online → cloud again; nothing replayed', async () => {
    const n = net(true);
    const h = await setup({ mode: 'cloud', network: n });
    n.set(false);
    await settle();
    expect(h.session.getSnapshot().processing.network).toBe('offline');
    h.session.sendText('good night');
    h.session.sendText('the parade was spectacular');
    await settle(40);
    expect(h.translate).not.toHaveBeenCalled();
    expect(segs(h).map((s) => [s.translated, s.untranslated ?? false])).toEqual([
      ['Buenas noches', false],
      ['the parade was spectacular', true],
    ]);
    n.set(true);
    await settle(40);
    expect(segs(h)).toHaveLength(2); // no replay of the untranslated phrase
    h.session.sendText('the parade was spectacular');
    await settle(40);
    expect(h.translate).toHaveBeenCalledTimes(1);
  });
});

describe('peer privacy request (priv1)', () => {
  it('a sender in on-device mode keeps their words on-device here too', async () => {
    const h = await setup({ mode: 'cloud' });
    h.call.receive(peerSeg({ id: 'a', original: 'el museo nuevo es precioso', translated: 'el museo nuevo es precioso', targetLang: 'es-ES', untranslated: true, priv: 'device' }));
    await settle(40);
    expect(h.translate).not.toHaveBeenCalled();
    const rec = h.session.getSnapshot().segments.find((s) => s.id === 'a')!;
    expect(rec.privateRequest).toBe(true);
    expect(h.playback.played.at(-1)!.ttsRoute!()).toBe('device-only');
    expect(h.session.getSnapshot().processing.peerPrivate).toBe(true);
  });

  it('ignored from a peer that did not negotiate priv1', async () => {
    const h = await setup({ mode: 'cloud', peerCaps: [] });
    h.call.receive(peerSeg({ id: 'a', original: 'hola amigo mío', translated: 'hola amigo mío', targetLang: 'es-ES', priv: 'device' }));
    await settle(40);
    expect(h.translate).toHaveBeenCalledTimes(1);
  });
});

describe('privacy lifecycle', () => {
  it('mode choice persists as a setting; nothing else is stored', async () => {
    const storage = memoryStorage();
    storage.setItem('talktual.consent.v1.browser-speech', 'granted');
    const a = await setup({ storage });
    a.session.setProcessingMode('local-only');
    expect(storage.getItem(PROCESSING_STORAGE_KEY)).toBe('local-only');
    a.session.dispose();
    const b = await setup({ storage });
    expect(b.session.getSnapshot().processing.mode).toBe('local-only');
    expect([...storage.data.keys()].sort()).toEqual([PROCESSING_STORAGE_KEY, 'talktual.consent.v1.browser-speech'].sort());
  });

  it('clearPrivateData cancels playback and clears caches, audit and intelligence context; session end clears caches', async () => {
    const h = await setup({ extra: { intelligence: true } });
    h.session.sendText('Tell Rose to meet us at Plaza Mayor');
    h.call.receive(peerSeg({ id: 'z', original: 'vale', translated: 'okay' }));
    await settle(40);
    expect(h.playback.active).toBe(true);
    expect(h.session.getSnapshot().intelligence.context?.turns).toBeGreaterThan(0);
    h.session.clearPrivateData();
    expect(h.playback.active).toBe(false);
    expect(h.clearCaches).toHaveBeenCalledTimes(1);
    expect(h.session.getSnapshot().intelligence.context?.turns).toBe(0);
    h.session.leave();
    expect(h.clearCaches).toHaveBeenCalledTimes(2);
  });

  it('diagnostics: route enums only — no words', async () => {
    const h = await setup({ mode: 'hybrid' });
    h.session.sendText('where is the train station');
    h.session.sendText('my secret password is tulip');
    await settle(40);
    const dump = JSON.stringify(h.session.getDiagnostics());
    for (const w of ['station', 'estación', 'secret', 'tulip']) expect(dump).not.toContain(w);
    expect(dump).toContain('"type":"route"');
  });

  it('typed input is bounded and ignored before the call is connected', async () => {
    const h = await setup({ mode: 'local-only' });
    h.session.sendText('   ');
    h.session.sendText('x'.repeat(2000));
    await settle(40);
    expect(segs(h)).toHaveLength(1);
    expect(segs(h)[0].original.length).toBe(500);
  });
});
