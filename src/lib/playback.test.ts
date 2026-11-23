import { describe, it, expect, vi, beforeEach } from 'vitest';
import { deferred, flush } from '../test/fakes';

// Speech synthesis fake: records utterances; the test decides when they end.
class FakeSynth {
  spoken: { text: string; lang: string; utt: { onend: null | (() => void); onerror: null | (() => void) } }[] = [];
  speak(utt: { text: string; lang: string; onend: null | (() => void); onerror: null | (() => void) }) {
    this.spoken.push({ text: utt.text, lang: utt.lang, utt });
  }
  cancel() {
    this.spoken.at(-1)?.utt.onerror?.();
  }
  getVoices() {
    return [];
  }
  finishLast() {
    this.spoken.at(-1)?.utt.onend?.();
  }
  failLast() {
    this.spoken.at(-1)?.utt.onerror?.();
  }
}

let synth: FakeSynth;
let fetchMock: ReturnType<typeof vi.fn>;

async function load() {
  vi.resetModules();
  return import('./textToSpeech');
}

function installSynth() {
  synth = new FakeSynth();
  Object.defineProperty(window, 'speechSynthesis', { value: synth, configurable: true });
  (globalThis as unknown as { SpeechSynthesisUtterance: unknown }).SpeechSynthesisUtterance = class {
    lang = '';
    voice: unknown = null;
    onend: null | (() => void) = null;
    onerror: null | (() => void) = null;
    constructor(public text: string) {}
  };
}

beforeEach(() => {
  installSynth();
  fetchMock = vi.fn(async () => new Response('{}', { status: 503 }));
  vi.stubGlobal('fetch', fetchMock);
  vi.spyOn(console, 'warn').mockImplementation(() => {});
});

function track(ctrl: { onStatus: (fn: (id: string, s: string, e?: string) => void) => void }) {
  const log: string[] = [];
  ctrl.onStatus((id, s, e) => log.push(`${id}:${s}${e ? `:${e}` : ''}`));
  return log;
}

describe('PlaybackController (M2.2)', () => {
  it('emits queued → started → ended per segment, with the engine that played it', async () => {
    const { PlaybackController } = await load();
    const c = new PlaybackController();
    const log = track(c);
    const p = c.enqueue({ segmentId: 'a', text: 'hola', lang: 'es-ES' });
    await vi.waitFor(() => expect(synth.spoken).toHaveLength(1));
    expect(c.active).toBe(true);
    synth.finishLast();
    await p;
    expect(log).toEqual(['a:queued', 'a:started', 'a:ended:browser']);
    expect(c.active).toBe(false);
  });

  it('bounded queue: beyond the limit the oldest waiting segment is dropped, never the playing one', async () => {
    const { PlaybackController } = await load();
    const c = new PlaybackController(2);
    const log = track(c);
    c.enqueue({ segmentId: 'p', text: 'playing', lang: 'en' });
    await vi.waitFor(() => expect(synth.spoken).toHaveLength(1));
    ['w1', 'w2', 'w3', 'w4'].forEach((id) => c.enqueue({ segmentId: id, text: id, lang: 'en' }));
    expect(c.depth).toBe(3); // playing + 2 waiting
    expect(log.filter((l) => l.endsWith(':dropped'))).toEqual(['w1:dropped', 'w2:dropped']);
    synth.finishLast();
    await vi.waitFor(() => expect(synth.spoken).toHaveLength(2));
    expect(synth.spoken[1].text).toBe('w3');
    c.cancelAll(); // don't leave w4 playing into the next test
  });

  it('Phase 4 age bound: a waiting item past notAfter is dropped instead of played late', async () => {
    const { PlaybackController } = await load();
    const c = new PlaybackController();
    const log = track(c);
    c.enqueue({ segmentId: 'p', text: 'playing', lang: 'en' });
    await vi.waitFor(() => expect(synth.spoken).toHaveLength(1));
    c.enqueue({ segmentId: 'old', text: 'old', lang: 'en', notAfter: Date.now() - 1 });
    c.enqueue({ segmentId: 'fresh', text: 'fresh', lang: 'en', notAfter: Date.now() + 60_000 });
    synth.finishLast();
    await vi.waitFor(() => expect(synth.spoken).toHaveLength(2));
    expect(synth.spoken[1].text).toBe('fresh');
    expect(log).toContain('old:dropped');
    c.cancelAll();
  });

  it('reports failed (not ended) when neither server audio nor a browser voice could play', async () => {
    const { PlaybackController } = await load();
    const c = new PlaybackController();
    const log = track(c);
    const p = c.enqueue({ segmentId: 'x', text: 'hola', lang: 'es' });
    await vi.waitFor(() => expect(synth.spoken).toHaveLength(1));
    synth.failLast(); // e.g. language-unavailable
    await p;
    expect(log).toEqual(['x:queued', 'x:started', 'x:failed']);
  });

  it('reports failed when the browser has no speech synthesis at all', async () => {
    const saved = Object.getOwnPropertyDescriptor(window, 'speechSynthesis');
    Reflect.deleteProperty(window, 'speechSynthesis');
    try {
      const { PlaybackController } = await load();
      const c = new PlaybackController();
      const log = track(c);
      await c.enqueue({ segmentId: 'y', text: 'hola', lang: 'es' });
      expect(log.at(-1)).toBe('y:failed');
    } finally {
      if (saved) Object.defineProperty(window, 'speechSynthesis', saved);
    }
  });

  it('cancelAll during the network request: aborts it, cancels everything, nothing starts afterwards', async () => {
    const pending = deferred<Response>();
    let signal: AbortSignal | undefined;
    fetchMock.mockImplementation((_u: string, init: RequestInit) => {
      signal = init.signal!;
      init.signal!.addEventListener('abort', () => pending.reject(new DOMException('aborted', 'AbortError')));
      return pending.promise;
    });
    const { PlaybackController } = await load();
    const c = new PlaybackController();
    const log = track(c);
    const ps = ['a', 'b', 'c'].map((id) => c.enqueue({ segmentId: id, text: id, lang: 'en' }));
    await vi.waitFor(() => expect(signal).toBeDefined());
    expect(c.cancelAll()).toBe(3);
    await Promise.all(ps);
    await flush(30);
    expect(signal!.aborted).toBe(true);
    expect(log.filter((l) => l.endsWith('cancelled')).sort()).toEqual(['a:cancelled', 'b:cancelled', 'c:cancelled']);
    expect(log.some((l) => l.endsWith(':ended') || l.endsWith(':failed'))).toBe(false);
    expect(synth.spoken).toHaveLength(0);
    expect(c.active).toBe(false);
  });

  it('cancelAll during browser speech: cancels synthesis; queue does not resume', async () => {
    const { PlaybackController } = await load();
    const c = new PlaybackController();
    const log = track(c);
    c.enqueue({ segmentId: 'a', text: 'a', lang: 'en' });
    c.enqueue({ segmentId: 'b', text: 'b', lang: 'en' });
    await vi.waitFor(() => expect(synth.spoken).toHaveLength(1));
    c.cancelAll();
    await flush(30);
    expect(synth.spoken).toHaveLength(1);
    expect(log).toContain('a:cancelled');
    expect(log).toContain('b:cancelled');
    // A new segment after cancel plays normally.
    const p = c.enqueue({ segmentId: 'c', text: 'c', lang: 'en' });
    await vi.waitFor(() => expect(synth.spoken).toHaveLength(2));
    synth.finishLast();
    await p;
    expect(log.at(-1)).toBe('c:ended:browser');
  });
});

describe('PCM streaming bounds (M2.2)', () => {
  it('never schedules more than PCM_MAX_AHEAD_S ahead of the audio clock (backpressure)', async () => {
    vi.useFakeTimers();
    try {
      let maxAhead = 0;
      const ctxRef: { ctx: { currentTime: number } | null } = { ctx: null };
      class FakeCtx {
        state = 'running';
        currentTime = 0;
        destination = {};
        constructor() {
          ctxRef.ctx = this;
        }
        createGain() {
          return { connect() {}, disconnect() {} };
        }
        createBuffer(_c: number, length: number, rate: number) {
          const data = new Float32Array(length);
          return { duration: length / rate, getChannelData: () => data };
        }
        createBufferSource() {
          const ctx = this;
          const node = {
            buffer: null as null | { duration: number },
            onended: null as null | (() => void),
            connect() {},
            start(at: number) {
              maxAhead = Math.max(maxAhead, at + node.buffer!.duration - ctx.currentTime);
            },
            stop() {},
          };
          return node;
        }
        async resume() {}
      }
      vi.stubGlobal('AudioContext', FakeCtx);
      // 30 chunks of 1 s of audio each at 1000 Hz (2000 bytes), delivered instantly.
      fetchMock.mockImplementation(async () => {
        const body = new ReadableStream({
          start(c) {
            for (let i = 0; i < 30; i++) c.enqueue(new Uint8Array(2000));
            c.close();
          },
        });
        return new Response(body, { status: 200, headers: { 'X-Audio-Format': 'pcm', 'X-Sample-Rate': '1000' } });
      });
      const { PlaybackController, PCM_MAX_AHEAD_S } = await load();
      const c = new PlaybackController();
      const done = c.enqueue({ segmentId: 'long', text: 'long', lang: 'en' });
      // Advance the audio clock with wall time.
      for (let t = 0; t < 40; t++) {
        await vi.advanceTimersByTimeAsync(1000);
        ctxRef.ctx!.currentTime += 1;
      }
      await vi.advanceTimersByTimeAsync(2000);
      await done;
      expect(maxAhead).toBeGreaterThan(PCM_MAX_AHEAD_S - 1);
      expect(maxAhead).toBeLessThanOrEqual(PCM_MAX_AHEAD_S + 1.1);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('browser playback restrictions (reliability fix)', () => {
  function suspendedCtx(opts: { resumes: boolean }) {
    const ref: { ctx: { state: string; currentTime: number; started: number } | null } = { ctx: null };
    class FakeCtx {
      state = 'suspended';
      currentTime = 0; // a suspended context's clock does not advance
      destination = {};
      started = 0;
      constructor() {
        ref.ctx = this;
      }
      createGain() {
        return { connect() {}, disconnect() {} };
      }
      createBuffer(_c: number, length: number, rate: number) {
        const data = new Float32Array(length);
        return { duration: length / rate, getChannelData: () => data };
      }
      createBufferSource() {
        const ctx = this;
        return { buffer: null, onended: null, connect() {}, start() { ctx.started++; }, stop() {} };
      }
      resume() {
        if (opts.resumes) {
          this.state = 'running';
          return Promise.resolve();
        }
        return new Promise<void>(() => {}); // no user gesture: stays pending forever
      }
    }
    vi.stubGlobal('AudioContext', FakeCtx);
    return ref;
  }
  const pcm = (seconds: number) =>
    fetchMock.mockImplementation(async () => {
      const body = new ReadableStream({
        start(c) {
          for (let i = 0; i < seconds; i++) c.enqueue(new Uint8Array(2000));
          c.close();
        },
      });
      return new Response(body, { status: 200, headers: { 'X-Audio-Format': 'pcm', 'X-Sample-Rate': '1000' } });
    });

  it('suspended AudioContext (no gesture): streamed server audio is NOT played silently — the browser voice speaks instead', async () => {
    const ref = suspendedCtx({ resumes: false });
    pcm(3);
    const { PlaybackController } = await load();
    const c = new PlaybackController();
    const log = track(c);
    const done = c.enqueue({ segmentId: 's', text: 'hola', lang: 'es-ES' });
    await vi.waitFor(() => expect(synth.spoken).toHaveLength(1), { timeout: 2000 });
    expect(ref.ctx!.started).toBe(0); // nothing scheduled into the dead context
    synth.finishLast();
    await done;
    expect(log).toEqual(['s:queued', 's:started', 's:ended:browser']);
  });

  it('a long clip into a suspended context no longer blocks the queue forever', async () => {
    suspendedCtx({ resumes: false });
    pcm(30); // 30 s of audio: previously stuck in the backpressure loop on a frozen clock
    const { PlaybackController } = await load();
    const c = new PlaybackController();
    c.enqueue({ segmentId: 'long', text: 'largo', lang: 'es-ES' });
    c.enqueue({ segmentId: 'next', text: 'siguiente', lang: 'es-ES' });
    await vi.waitFor(() => expect(synth.spoken.map((x) => x.text)).toContain('largo'), { timeout: 2000 });
    synth.finishLast();
    await vi.waitFor(() => expect(synth.spoken.map((x) => x.text)).toContain('siguiente'), { timeout: 2000 });
  });

  it('a context that can resume is resumed and used for server audio', async () => {
    const ref = suspendedCtx({ resumes: true });
    pcm(1);
    const { PlaybackController } = await load();
    const c = new PlaybackController();
    const log = track(c);
    await c.enqueue({ segmentId: 'ok', text: 'hola', lang: 'es-ES' });
    expect(ref.ctx!.started).toBeGreaterThan(0);
    expect(synth.spoken).toHaveLength(0);
    expect(log.at(-1)).toBe('ok:ended:server');
  });
});

describe('capture adapter (M2.2)', () => {
  it('reports device loss when the track ends unexpectedly, not on our own release', async () => {
    const listeners: Record<string, () => void> = {};
    const track = {
      readyState: 'live',
      enabled: true,
      stop() {
        this.readyState = 'ended';
      },
      addEventListener: (ev: string, fn: () => void) => (listeners[ev] = fn),
    };
    const stream = { getAudioTracks: () => [track], getTracks: () => [track] };
    Object.defineProperty(navigator, 'mediaDevices', { value: { getUserMedia: async () => stream }, configurable: true });
    vi.resetModules();
    const audio = await import('./audio');
    const lost = vi.fn();
    audio.captureAdapter.onDeviceLost(lost);
    await audio.getMic();
    listeners.ended();
    expect(lost).toHaveBeenCalledTimes(1);
    // After loss the next getMic re-acquires.
    await audio.getMic();
    audio.releaseMic(); // our own stop: no 'ended' event is dispatched by browsers
    expect(lost).toHaveBeenCalledTimes(1);
  });

  it('permission query maps states and degrades to unknown where unsupported', async () => {
    vi.resetModules();
    const audio = await import('./audio');
    Object.defineProperty(navigator, 'permissions', { value: { query: async () => ({ state: 'denied' }) }, configurable: true });
    expect(await audio.micPermission()).toBe('denied');
    Object.defineProperty(navigator, 'permissions', { value: { query: async () => ({ state: 'granted' }) }, configurable: true });
    expect(await audio.micPermission()).toBe('granted');
    Object.defineProperty(navigator, 'permissions', {
      value: {
        query: async () => {
          throw new TypeError('microphone is not a valid permission name');
        },
      },
      configurable: true,
    });
    expect(await audio.micPermission()).toBe('unknown');
  });
});
