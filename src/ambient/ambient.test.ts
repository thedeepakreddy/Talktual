import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { StablePrefixSegmenter, commonPrefix, type AmbientUnit } from './stablePrefix';
import { guessLanguage, LanguageMismatchDetector } from './langGuess';
import { AmbientSession, type AmbientDeps } from './AmbientSession';
import { ambientConsentKey } from './disclosure';
import { CHROME_LIKE, FakeAsr, FakeCapture, FakePlayback, memoryStorage } from '../test/engineFakes';
import type { TranslationRequest, TranslationResult } from '../engine/ports';

// ---------------------------------------------------------------------------
// Stable-prefix segmentation
// ---------------------------------------------------------------------------
describe('StablePrefixSegmenter', () => {
  let t = 0;
  let units: AmbientUnit[] = [];
  const mk = (cfg = {}) => new StablePrefixSegmenter('hu-HU', (u) => units.push(u), () => t, { stableMs: 700, minUpdates: 3, minEarlyWords: 6, maxOpenMs: 5000, dupWindowMs: 4000, lowConfidence: 0.45, maxHistory: 40, ...cfg });
  beforeEach(() => {
    t = 1000;
    units = [];
  });

  it('a final with no interims is one unit', () => {
    const s = mk();
    s.final('Jó napot kívánok');
    expect(units.map((u) => [u.text, u.kind])).toEqual([['Jó napot kívánok', 'final']]);
  });

  it('a long run-on commits its stable prefix early, then only the remainder at the final — no repeats', () => {
    const s = mk();
    const words = 'holnap reggel nyolc órakor találkozunk a pályaudvar előtt és utána elmegyünk a piacra'.split(' ');
    for (let n = 2; n <= words.length; n++) {
      s.partial(words.slice(0, n).join(' '));
      t += 300;
    }
    expect(units.length).toBeGreaterThanOrEqual(1);
    expect(units[0].kind).toBe('early');
    s.final(words.join(' '));
    const all = units.map((u) => u.text).join(' ');
    expect(all).toBe(words.join(' ')); // every word exactly once, in order
    expect(units.at(-1)!.kind).toBe('final');
  });

  it('nothing is committed early from a hypothesis that keeps changing', () => {
    const s = mk();
    const variants = ['egy kettő három négy öt hat hét', 'egy ketten három négy öt hat hét', 'egy kettő hárman négy öt hat hét'];
    for (let i = 0; i < 9; i++) {
      s.partial(variants[i % 3] + ' nyolc');
      t += 150;
    }
    expect(units.filter((u) => u.kind === 'early')).toHaveLength(0);
  });

  it('a revision of already-committed words is flagged, and the final remainder is computed against the final', () => {
    const s = mk();
    const a = 'a vonat tíz perc múlva indul a második vágányról';
    for (let i = 0; i < 6; i++) {
      s.partial(a);
      t += 300;
    }
    expect(units).toHaveLength(1);
    const early = units[0].text.split(' ').length;
    // The recognizer now says the 2nd word differently.
    s.final('a busz tíz perc múlva indul a második vágányról');
    const last = units.at(-1)!;
    expect(last.revised).toBe(true);
    expect(last.text.split(' ').length).toBe(9 - 1); // everything after the first shared word
    expect(early).toBeGreaterThanOrEqual(3);
  });

  it('fillers and immediate duplicates are dropped; reset forgets interims', () => {
    const s = mk();
    s.final('um');
    s.final('Köszönöm szépen');
    t += 500;
    s.final('köszönöm szépen');
    expect(units.map((u) => u.text)).toEqual(['Köszönöm szépen']);
    s.partial('ez csak egy félbehagyott mondat volt itt most');
    s.reset();
    t += 5000;
    s.final('új mondat');
    expect(units.at(-1)!.text).toBe('új mondat');
    expect(units.at(-1)!.revised).toBe(false);
  });

  it('low confidence marks a unit uncertain (never drops or rewrites it)', () => {
    const s = mk();
    s.final('talán holnap', 0.2);
    expect(units[0]).toMatchObject({ text: 'talán holnap', uncertain: true });
  });

  it('commonPrefix ignores case and punctuation', () => {
    expect(commonPrefix(['Hello,', 'world'], ['hello', 'World!', 'x'])).toBe(2);
  });
});

describe('language check', () => {
  it('recognises Hungarian and English function words', () => {
    expect(guessLanguage('nem tudom hogy hol van az állomás')?.lang).toBe('hu');
    expect(guessLanguage('I think that the station is over there')?.lang).toBe('en');
    expect(guessLanguage('jó')).toBeNull();
  });
  it('flags a clear mismatch only after enough words; never for the expected language', () => {
    const d = new LanguageMismatchDetector('hu-HU');
    expect(d.add('is it far from here')).toBeNull();
    expect(d.add('I think the station is just over there and it is not far')).toBe('en');
    const h = new LanguageMismatchDetector('hu-HU');
    expect(h.add('nem tudom hogy hol van az állomás de azt hiszem hogy itt van a közelben')).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// AmbientSession
// ---------------------------------------------------------------------------
interface Deferred {
  req: TranslationRequest;
  resolve: (text?: string) => void;
  fail: () => void;
}

function harness(opts: { consent?: boolean; browser?: Partial<typeof CHROME_LIKE>; processing?: AmbientDeps['processing']; headphones?: boolean; speak?: boolean; manual?: boolean; limits?: Record<string, number>; audioSession?: { type: string } | null } = {}) {
  FakeAsr.all = [];
  FakeAsr.denyNext = false;
  const storage = memoryStorage();
  if (opts.consent !== false) storage.setItem(ambientConsentKey('browser-speech'), 'granted');
  const playback = new FakePlayback();
  const capture = new FakeCapture();
  const calls: TranslationRequest[] = [];
  const deferred: Deferred[] = [];
  const translate = async (req: TranslationRequest): Promise<TranslationResult> => {
    calls.push(req);
    if (!opts.manual) return { segmentId: req.segmentId, status: 'translated', text: `EN(${req.text})`, attempts: 1, latencyMs: 5 };
    return new Promise((res) => {
      const onAbort = () => res({ segmentId: req.segmentId, status: 'cancelled', text: req.text, attempts: 1, latencyMs: 0 });
      req.signal?.addEventListener('abort', onAbort, { once: true });
      deferred.push({
        req,
        resolve: (text) => res({ segmentId: req.segmentId, status: 'translated', text: text ?? `EN(${req.text})`, attempts: 1, latencyMs: 5 }),
        fail: () => res({ segmentId: req.segmentId, status: 'failed', text: req.text, error: { code: 'http', message: 'x' }, attempts: 1, latencyMs: 5 }),
      });
    });
  };
  let hidden = false;
  const visSubs = new Set<() => void>();
  const deps: AmbientDeps = {
    createAsr: (lang, mode, cb, o) => {
      const a = new FakeAsr('webspeech', lang, mode, cb);
      (a as unknown as { opts: unknown }).opts = o;
      return a;
    },
    translate,
    playback,
    capture,
    browser: { ...CHROME_LIKE, ...opts.browser },
    storage,
    getServerCapabilities: async () => ({ tts: false, stt: false, translator: 'mymemory', liveCaptions: false }),
    clearCaches: vi.fn(),
    processing: opts.processing,
    visibility: { hidden: () => hidden, subscribe: (fn) => (visSubs.add(fn), () => visSubs.delete(fn)) },
    audioSession: opts.audioSession ?? null,
  };
  const s = new AmbientSession({ from: 'hu-HU', to: 'en-US', headphones: opts.headphones, speak: opts.speak }, deps, opts.limits);
  const asr = () => FakeAsr.all.at(-1)!;
  const setHidden = (v: boolean) => {
    hidden = v;
    visSubs.forEach((f) => f());
  };
  return { s, deps, storage, playback, capture, calls, deferred, asr, setHidden };
}
const flush = async (n = 10) => {
  for (let i = 0; i < n; i++) await Promise.resolve();
};

describe('AmbientSession', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('first use asks for consent; nothing listens until it is granted; then continuous recognition in the chosen language', async () => {
    const h = harness({ consent: false });
    h.s.start();
    expect(h.s.getSnapshot().consent.prompt).toBe(true);
    expect(h.s.getSnapshot().consent.disclosure?.points.join(' ')).toMatch(/people near you/);
    expect(FakeAsr.all).toHaveLength(0);
    h.s.declineConsent();
    expect(FakeAsr.all).toHaveLength(0);
    expect(h.s.getSnapshot().phase).toBe('idle');
    h.s.start();
    h.s.grantConsent();
    await flush();
    expect(FakeAsr.all).toHaveLength(1);
    expect(h.asr()).toMatchObject({ lang: 'hu-HU', mode: 'continuous', active: true });
    expect(h.s.getSnapshot()).toMatchObject({ phase: 'listening', capturing: true });
    expect(h.storage.data.get(ambientConsentKey('browser-speech'))).toBe('granted');
  });

  it('no speech recognition in this browser → unavailable, explained, nothing created', () => {
    const h = harness({ browser: { webSpeech: false } });
    h.s.start();
    expect(FakeAsr.all).toHaveLength(0);
    expect(h.s.getSnapshot()).toMatchObject({ phase: 'unavailable', notice: { code: 'no-speech-recognition' } });
  });

  it('heard → translated → spoken, strictly in the order heard even when translations finish out of order', async () => {
    const h = harness({ manual: true, headphones: true });
    h.s.start();
    await flush();
    h.asr().say('első mondat');
    h.asr().say('második mondat');
    expect(h.deferred).toHaveLength(2);
    h.deferred[1].resolve(); // second finishes first
    await flush();
    expect(h.playback.played).toHaveLength(0);
    h.deferred[0].resolve();
    await flush();
    expect(h.playback.played.map((p) => p.text)).toEqual(['EN(első mondat)']);
    h.playback.finish();
    expect(h.playback.played.map((p) => p.text)).toEqual(['EN(első mondat)', 'EN(második mondat)']);
    expect(h.playback.played.every((p) => p.lang === 'en-US' && p.volume === 0.8)).toBe(true);
    const lines = h.s.getSnapshot().lines;
    expect(lines.map((l) => [l.original, l.translation])).toEqual([
      ['első mondat', 'EN(első mondat)'],
      ['második mondat', 'EN(második mondat)'],
    ]);
  });

  it('a failed translation is shown as heard, not spoken, and does not block later phrases', async () => {
    const h = harness({ manual: true, headphones: true });
    h.s.start();
    await flush();
    h.asr().say('egy');
    h.asr().say('kettő');
    h.deferred[0].fail();
    h.deferred[1].resolve();
    await flush();
    const [a, b] = h.s.getSnapshot().lines;
    expect(a).toMatchObject({ state: 'untranslated', speech: 'skipped' });
    expect(a.note).toMatch(/Not translated/);
    expect(b.state).toBe('translated');
    expect(h.playback.played.map((p) => p.text)).toEqual(['EN(kettő)']);
  });

  it('without headphones: capture stops while our speech plays (no feedback loop) and resumes after', async () => {
    const h = harness();
    h.s.start();
    await flush();
    const first = h.asr();
    first.say('jó reggelt');
    await flush();
    expect(h.playback.playingItem?.text).toBe('EN(jó reggelt)');
    expect(first.active).toBe(false);
    expect(first.aborts).toBe(1);
    expect(h.capture.releases).toBeGreaterThan(0);
    expect(h.s.getSnapshot()).toMatchObject({ phase: 'listening', capturing: false, heldForSpeech: true });
    h.playback.finish();
    await vi.advanceTimersByTimeAsync(400);
    expect(FakeAsr.all).toHaveLength(2);
    expect(h.asr().active).toBe(true);
    expect(h.s.getSnapshot()).toMatchObject({ capturing: true, heldForSpeech: false });
  });

  it('stop, pause or dispose while speech holds capture never turns the microphone back on', async () => {
    for (const action of ['stop', 'pause', 'dispose'] as const) {
      const h = harness();
      h.s.start();
      await flush();
      h.asr().say('jó reggelt');
      await flush();
      expect(h.s.getSnapshot().heldForSpeech).toBe(true);
      h.s[action]();
      h.playback.finish();
      await vi.advanceTimersByTimeAsync(2000);
      expect(FakeAsr.all).toHaveLength(1);
      expect(FakeAsr.all.some((a) => a.active)).toBe(false);
    }
  });

  it('with headphones: keeps listening while speaking', async () => {
    const h = harness({ headphones: true });
    h.s.start();
    await flush();
    h.asr().say('jó reggelt');
    await flush();
    expect(h.playback.playingItem).not.toBeNull();
    expect(h.asr().active).toBe(true);
    expect(FakeAsr.all).toHaveLength(1);
  });

  it('echo guard: our own translated speech recognised by the mic is dropped', async () => {
    const h = harness({ headphones: false });
    h.s.start();
    await flush();
    h.asr().say('köszönöm');
    await flush();
    h.playback.finish();
    await vi.advanceTimersByTimeAsync(400);
    h.asr().say('EN(köszönöm)'); // the speaker's output, re-heard
    await flush();
    expect(h.s.getSnapshot().lines).toHaveLength(1);
    expect(h.s.diagnostics().counters.echo_dropped).toBe(1);
  });

  it('text-only: nothing is spoken; turning speech off stops what is playing', async () => {
    const h = harness({ headphones: true });
    h.s.start();
    await flush();
    h.asr().say('egy');
    await flush();
    expect(h.playback.playingItem).not.toBeNull();
    h.s.setSpeak(false);
    expect(h.playback.cancelled).toBe(1);
    h.asr().say('kettő');
    await flush();
    expect(h.playback.played).toHaveLength(1);
    expect(h.s.getSnapshot().lines.at(-1)).toMatchObject({ translation: 'EN(kettő)', speech: 'off' });
  });

  it('late translations are shown but not spoken', async () => {
    const h = harness({ manual: true, headphones: true });
    h.s.start();
    await flush();
    h.asr().say('lassú');
    await vi.advanceTimersByTimeAsync(10_000);
    h.deferred[0].resolve();
    await flush();
    expect(h.playback.played).toHaveLength(0);
    expect(h.s.getSnapshot().lines[0]).toMatchObject({ state: 'translated', speech: 'skipped' });
    expect(h.s.getSnapshot().lines[0].note).toMatch(/too late/);
  });

  it('backlog is bounded: when the room outpaces translation, the oldest waiting phrases are skipped', async () => {
    const h = harness({ manual: true, headphones: true, limits: { maxPending: 3, maxInFlight: 1 } });
    h.s.start();
    await flush();
    for (let i = 0; i < 8; i++) h.asr().say(`mondat ${i} itt`);
    const lines = h.s.getSnapshot().lines;
    expect(h.deferred).toHaveLength(1); // only one in flight
    expect(lines.filter((l) => l.state === 'skipped')).toHaveLength(4);
    expect(lines.filter((l) => l.state === 'translating')).toHaveLength(4); // 1 in flight + 3 waiting
  });

  it('pause aborts recognition at once; resume starts a fresh recognizer; stop cancels speech and translations', async () => {
    const h = harness({ manual: true, headphones: true });
    h.s.start();
    await flush();
    h.asr().say('egy', false);
    h.s.pause();
    expect(h.asr().active).toBe(false);
    expect(h.asr().aborts).toBe(1);
    expect(h.s.getSnapshot()).toMatchObject({ phase: 'paused', pauseReason: 'user', capturing: false, partial: '' });
    h.s.resume();
    await flush();
    expect(FakeAsr.all).toHaveLength(2);
    h.asr().say('kettő');
    expect(h.deferred).toHaveLength(1);
    h.s.stop();
    await flush();
    expect(h.deferred[0].req.signal?.aborted).toBe(true);
    expect(h.s.getSnapshot()).toMatchObject({ phase: 'stopped', capturing: false });
    expect(h.s.getSnapshot().lines[0].state).toBe('skipped');
  });

  it('clear drops all lines and caches; nothing about the room is ever written to storage', async () => {
    const h = harness({ headphones: true });
    h.s.start();
    await flush();
    h.asr().say('titkos mondat a kávézóban');
    await flush();
    expect(h.s.getSnapshot().lines).toHaveLength(1);
    h.s.clear();
    expect(h.s.getSnapshot().lines).toHaveLength(0);
    expect(h.deps.clearCaches).toHaveBeenCalled();
    expect([...h.storage.data.keys()]).toEqual([ambientConsentKey('browser-speech')]);
    expect(JSON.stringify(h.s.diagnostics())).not.toMatch(/titkos|kávézó|EN\(/);
  });

  it('leaving the screen pauses listening and it does not resume by itself', async () => {
    const h = harness();
    h.s.start();
    await flush();
    h.setHidden(true);
    expect(h.s.getSnapshot()).toMatchObject({ phase: 'paused', pauseReason: 'hidden', capturing: false });
    h.setHidden(false);
    await flush();
    expect(h.s.getSnapshot().phase).toBe('paused');
    expect(FakeAsr.all).toHaveLength(1);
  });

  it('watchdog: a silent recognizer is restarted, bounded, then a visible stalled state with Resume', async () => {
    const h = harness({ limits: { watchdogMs: 6000, maxRestartsPerMinute: 2 } });
    h.s.start();
    await flush();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(FakeAsr.all.length).toBe(2);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(FakeAsr.all.length).toBe(3);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(h.s.getSnapshot()).toMatchObject({ phase: 'stalled', capturing: false });
    expect(FakeAsr.all.every((a) => !a.active)).toBe(true);
    h.s.resume();
    await flush();
    expect(h.s.getSnapshot().phase).toBe('listening');
  });

  it('permission denied → unavailable with how to fix it', async () => {
    const h = harness();
    FakeAsr.denyNext = true;
    h.s.start();
    await flush();
    expect(h.s.getSnapshot()).toMatchObject({ phase: 'unavailable', notice: { code: 'permission-denied' } });
  });

  it('microphone lost → paused with a notice; Resume retries', async () => {
    const h = harness();
    h.s.start();
    await flush();
    h.capture.loseDevice();
    expect(h.s.getSnapshot()).toMatchObject({ phase: 'paused', pauseReason: 'mic', notice: { code: 'device-lost' } });
    h.s.resume();
    await flush();
    expect(h.s.getSnapshot().phase).toBe('listening');
  });

  it('WebKit audio session is set to play-and-record while listening and restored after', async () => {
    const audioSession = { type: 'auto' };
    const h = harness({ audioSession });
    h.s.start();
    await flush();
    expect(audioSession.type).toBe('play-and-record');
    h.s.pause();
    expect(audioSession.type).toBe('auto');
  });

  it('lines are bounded', async () => {
    const h = harness({ speak: false, limits: { maxLines: 5 } });
    h.s.start();
    await flush();
    for (let i = 0; i < 12; i++) {
      h.asr().say(`sor ${i} van itt`);
      await vi.advanceTimersByTimeAsync(5000);
    }
    expect(h.s.getSnapshot().lines).toHaveLength(5);
    expect(h.s.getSnapshot().lines[0].original).toBe('sor 7 van itt');
  });

  it('dispose releases the microphone and forgets the transcript', async () => {
    const h = harness();
    h.s.start();
    await flush();
    h.asr().say('valami');
    await flush();
    h.s.dispose();
    expect(h.asr().active).toBe(false);
    expect(h.s.getSnapshot().lines).toHaveLength(0);
  });
});

describe('AmbientSession × Phase 6 processing policy', () => {
  const network = (online = true) => {
    const subs = new Set<() => void>();
    let on = online;
    return { onLine: () => on, subscribe: (f: () => void) => (subs.add(f), () => subs.delete(f)), set: (v: boolean) => ((on = v), subs.forEach((f) => f())) };
  };
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('On-device only with no on-device Hungarian recognizer: listening is unavailable and nothing reaches the cloud', async () => {
    const h = harness({ processing: { mode: 'local-only', network: network() } });
    h.s.start();
    await flush();
    expect(FakeAsr.all).toHaveLength(0);
    expect(h.s.getSnapshot()).toMatchObject({ phase: 'unavailable', notice: { code: 'asr-local-only' } });
    expect(h.s.getSnapshot().processing).toMatchObject({ mode: 'local-only', asr: 'unavailable' });
    expect(h.calls).toHaveLength(0);
  });

  it('switching to On-device while listening pauses capture and aborts cloud translations in flight', async () => {
    const h = harness({ manual: true, headphones: true, processing: { mode: 'cloud', network: network() } });
    h.s.start();
    await flush();
    h.asr().say('felhő');
    expect(h.deferred).toHaveLength(1);
    h.s.setProcessingMode('local-only');
    await flush();
    expect(h.deferred[0].req.signal?.aborted).toBe(true);
    expect(h.asr().active).toBe(false);
    expect(h.s.getSnapshot()).toMatchObject({ phase: 'paused', pauseReason: 'policy' });
    // Untranslated, never sent elsewhere afterwards.
    await flush();
    expect(h.calls).toHaveLength(1);
    expect(h.s.getSnapshot().lines[0].state).not.toBe('translated');
  });

  it('offline pauses listening; back online resumes it (the user had started it)', async () => {
    const net = network();
    const h = harness({ processing: { mode: 'cloud', network: net } });
    h.s.start();
    await flush();
    net.set(false);
    expect(h.s.getSnapshot()).toMatchObject({ phase: 'paused', pauseReason: 'offline' });
    expect(h.asr().active).toBe(false);
    net.set(true);
    await flush();
    expect(h.s.getSnapshot().phase).toBe('listening');
    expect(FakeAsr.all).toHaveLength(2);
  });

  it('cloud mode records only enum audit entries (no text)', async () => {
    const h = harness({ headphones: true, processing: { mode: 'cloud', network: network() } });
    h.s.start();
    await flush();
    h.asr().say('magánbeszélgetés');
    await flush();
    expect(h.s.getSnapshot().lines[0].location).toBe('cloud');
    expect(JSON.stringify(h.s.diagnostics())).not.toMatch(/magán/);
  });
});
