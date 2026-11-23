import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { ValidationRecorder, coarseBrowser, validationEnabled } from './validation';
import { AmbientSession, type AmbientDeps, type DeviceSource } from './AmbientSession';
import { ambientConsentKey } from './disclosure';
import { CHROME_LIKE, FakeAsr, FakeCapture, FakePlayback, memoryStorage } from '../test/engineFakes';

describe('ValidationRecorder', () => {
  it('separates measured, engine-reported and human-marked timings, with p50/p90/p95', () => {
    let t = 0;
    const rec = new ValidationRecorder({ now: () => t });
    for (let i = 0; i < 10; i++) {
      const id = `x${i}`;
      t += 100;
      rec.partial();
      t += 900;
      rec.speech(false); // engine says speech ended
      t += 50;
      rec.marker(); // tester tapped
      t += 250;
      rec.final();
      t += 10;
      rec.commit(id, 'final');
      rec.translateReq(id);
      t += 200 + i * 10;
      rec.translateRes(id, 'translated', 'cloud');
      rec.queueAdmit(id, 1);
      t += 30;
      rec.playback(id, 'started');
      t += 800;
      rec.playback(id, 'ended');
    }
    const s = rec.summary({ browser: 'chrome', mobile: false });
    expect(s.measured.translate_ms).toMatchObject({ n: 10, p50: 250, p90: 290, p95: 290, max: 290 });
    expect(s.measured.recognizer_final_to_commit_ms?.p50).toBe(10);
    expect(s.engineReported.speech_end_to_final_ms?.p50).toBe(300);
    expect(s.humanMarked.mark_to_audio_start_ms?.n).toBe(10);
    expect(s.humanMarked.mark_to_audio_start_ms!.p50).toBeGreaterThan(s.measured.translated_to_audio_start_ms!.p50);
    expect(s.reliability).toMatchObject({ units: 10, finals_without_partials: 0, max_queue_depth: 1 });
  });

  it('counts missing finals, restarts, drops and interruptions; route changes carry counts only', () => {
    let t = 0;
    const rec = new ValidationRecorder({ now: () => (t += 10) });
    rec.partial();
    rec.asrStop('pause-user'); // utterance never finalized
    rec.final(); // final with no interim
    rec.asrRestart('silent');
    rec.queueDrop('a', 'late');
    rec.interruption('hidden');
    rec.routeChange(2, 3);
    const s = rec.summary();
    expect(s.reliability).toMatchObject({ utterances_without_final: 1, finals_without_partials: 1, asr_restarts: 1, queue_drops: 1, interruptions: { hidden: 1 }, route_changes: 1 });
    expect(s.events.find((e) => e.k === 'route_change')?.v).toBe('in2_out3');
  });

  it('is bounded and clearable', () => {
    const rec = new ValidationRecorder({ maxEvents: 50, maxUnits: 5 });
    for (let i = 0; i < 200; i++) rec.commit(`u${i}`, 'final');
    const s = rec.summary();
    expect(s.events.length).toBe(50);
    expect(s.reliability.units).toBeLessThanOrEqual(5);
    rec.clear();
    expect(rec.summary().events).toHaveLength(0);
  });

  it('opt-in and coarse environment only', () => {
    expect(validationEnabled('?validate=1')).toBe(true);
    expect(validationEnabled('?debug=1')).toBe(true);
    expect(validationEnabled('')).toBe(false);
    expect(validationEnabled('?validate=0')).toBe(false);
    expect(coarseBrowser('Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1')).toEqual({ browser: 'safari', mobile: true });
    expect(coarseBrowser('Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36')).toEqual({ browser: 'chrome', mobile: false });
  });
});

describe('AmbientSession × ValidationRecorder', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  function harness() {
    FakeAsr.all = [];
    const storage = memoryStorage();
    storage.setItem(ambientConsentKey('browser-speech'), 'granted');
    const playback = new FakePlayback();
    const subs = new Set<() => void>();
    let counts = { inputs: 1, outputs: 1 };
    const devices: DeviceSource = { counts: async () => counts, subscribe: (fn) => (subs.add(fn), () => subs.delete(fn)) };
    const rec = new ValidationRecorder({ now: () => Date.now() });
    const deps: AmbientDeps = {
      createAsr: (lang, mode, cb) => new FakeAsr('webspeech', lang, mode, cb),
      translate: async (req) => ({ segmentId: req.segmentId, status: 'translated', text: `EN(${req.text})`, attempts: 1, latencyMs: 1 }),
      playback,
      capture: new FakeCapture(),
      browser: CHROME_LIKE,
      storage,
      getServerCapabilities: async () => ({ tts: false, stt: false, translator: 'mymemory', liveCaptions: false }),
      visibility: null,
      audioSession: null,
      recorder: rec,
      devices,
    };
    const s = new AmbientSession({ from: 'hu-HU', to: 'en-US', headphones: true }, deps);
    const fire = (c: typeof counts) => {
      counts = c;
      subs.forEach((f) => f());
    };
    return { s, rec, playback, fire };
  }
  const flush = async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  };

  it('records the whole stage timeline without any speech content', async () => {
    const h = harness();
    h.s.start();
    await flush();
    const asr = FakeAsr.all.at(-1)!;
    asr.cb.onSpeechActivity?.(true);
    asr.say('titkos', false);
    await vi.advanceTimersByTimeAsync(400);
    asr.cb.onSpeechActivity?.(false);
    h.rec.marker();
    await vi.advanceTimersByTimeAsync(200);
    asr.say('titkos mondat a kávézóban', true);
    await flush();
    await vi.advanceTimersByTimeAsync(300);
    h.playback.finish();
    h.s.pause();
    const sum = h.rec.summary({ browser: 'chrome', mobile: true });
    expect(sum.measured.translate_ms?.n).toBe(1);
    expect(sum.measured.recognizer_final_to_audio_start_ms?.n).toBe(1);
    expect(sum.engineReported.speech_end_to_audio_start_ms?.n).toBe(1);
    expect(sum.humanMarked.mark_to_audio_start_ms?.n).toBe(1);
    expect(sum.reliability.interruptions).toEqual({ user: 1 });
    const json = JSON.stringify(sum);
    expect(json).not.toMatch(/titkos|mondat|kávézó|EN\(|hu-HU|en-US/);
  });

  it('device changes are recorded as counts and shown briefly while listening', async () => {
    const h = harness();
    h.s.start();
    await flush();
    h.fire({ inputs: 2, outputs: 2 }); // e.g. AirPods connected
    await flush();
    expect(h.s.getSnapshot().notice?.code).toBe('route-change');
    expect(h.rec.summary().reliability.route_changes).toBe(1);
    await vi.advanceTimersByTimeAsync(8500);
    expect(h.s.getSnapshot().notice).toBeNull();
  });

  it('teardown is recorded on dispose', async () => {
    const h = harness();
    h.s.start();
    await flush();
    h.s.dispose();
    expect(h.rec.summary().counters.teardown).toBe(1);
  });
});
