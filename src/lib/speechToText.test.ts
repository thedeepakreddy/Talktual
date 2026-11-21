import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { deferred, flush } from '../test/fakes';

const micDeferred = { current: deferred<MediaStream>() };
const micEnabled = vi.fn();
vi.mock('./audio', () => ({ getMic: () => micDeferred.current.promise, setMicEnabled: (on: boolean) => micEnabled(on) }));

const liveStream = {
  getAudioTracks: () => [{ readyState: 'live' }],
  getTracks: () => [],
} as unknown as MediaStream;

class FakeRecorder {
  static instances: FakeRecorder[] = [];
  static isTypeSupported = () => true;
  state: 'inactive' | 'recording' = 'inactive';
  mimeType = 'audio/webm';
  ondataavailable: ((e: { data: Blob }) => void) | null = null;
  onstop: (() => void) | null = null;
  constructor() {
    FakeRecorder.instances.push(this);
  }
  start() {
    this.state = 'recording';
  }
  stop() {
    this.state = 'inactive';
    this.ondataavailable?.({ data: new Blob([new Uint8Array(5000)]) });
    this.onstop?.();
  }
}

const cb = () => ({ onFinal: vi.fn(), onPartial: vi.fn(), onSpeechActivity: vi.fn(), onError: vi.fn(), onEnd: vi.fn() });
const texts = (fn: ReturnType<typeof vi.fn>) => fn.mock.calls.map((c) => (c[0] as { text: string }).text);
type Call = [string, RequestInit];

beforeEach(() => {
  FakeRecorder.instances = [];
  micEnabled.mockClear();
  micDeferred.current = deferred<MediaStream>();
  vi.stubGlobal('MediaRecorder', FakeRecorder);
  delete (window as unknown as Record<string, unknown>).webkitSpeechRecognition;
  delete (window as unknown as Record<string, unknown>).SpeechRecognition;
});
afterEach(() => vi.useRealTimers());

describe('server-STT adapter (batch)', () => {
  it('records between start and stop, posts it, emits a final segment with metadata, then ends', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ text: ' hola ' }), { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);
    const { createAsr } = await import('./speechToText');
    const c = cb();
    const r = createAsr('es-ES', 'push-to-talk', c);
    expect(r.engine).toBe('server-stt');
    expect(r.route).toBe('server-stt');
    micDeferred.current.resolve(liveStream);
    await r.start();
    expect(FakeRecorder.instances.some((x) => x.state === 'recording')).toBe(true);
    expect(micEnabled).toHaveBeenLastCalledWith(true);
    r.stop();
    await vi.waitFor(() => expect(c.onFinal).toHaveBeenCalled());
    const seg = c.onFinal.mock.calls[0][0];
    expect(seg).toMatchObject({ text: 'hola', lang: 'es-ES', engine: 'server-stt' });
    expect(seg.endedAt).toBeGreaterThanOrEqual(seg.startedAt);
    expect(((fetchMock.mock.calls as unknown as Call[])[0][1].body as FormData).get('language_code')).toBe('es');
    await vi.waitFor(() => expect(c.onEnd).toHaveBeenCalled());
    expect(r.active).toBe(false);
    expect(micEnabled).toHaveBeenLastCalledWith(false); // mic gated off between pushes
  });

  it('never acquires the mic or records until start()', async () => {
    const { createAsr } = await import('./speechToText');
    createAsr('en-US', 'push-to-talk', cb());
    micDeferred.current.resolve(liveStream);
    await flush(20);
    expect(FakeRecorder.instances).toHaveLength(0);
  });

  it('DEF-C08 (kept): a tap released before the mic is ready never starts a runaway recording', async () => {
    vi.stubGlobal('fetch', vi.fn());
    const { createAsr } = await import('./speechToText');
    const c = cb();
    const r = createAsr('en-US', 'push-to-talk', c);
    const started = r.start(); // mic prompt still open
    r.stop(); // quick tap
    micDeferred.current.resolve(liveStream);
    await started;
    await flush(20);
    expect(FakeRecorder.instances).toHaveLength(0);
    expect(r.active).toBe(false);
    expect(c.onEnd).toHaveBeenCalled();
  });

  it('DEF-C08 (kept): abort mid-push discards the audio (no upload, no segment)', async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    const { createAsr } = await import('./speechToText');
    const c = cb();
    const r = createAsr('en-US', 'push-to-talk', c);
    micDeferred.current.resolve(liveStream);
    await r.start();
    r.abort();
    await flush(20);
    expect(fetchMock).not.toHaveBeenCalled();
    expect(c.onFinal).not.toHaveBeenCalled();
    expect(micEnabled).toHaveBeenLastCalledWith(false);
  });

  it('abort during the upload drops the late result', async () => {
    const res = deferred<Response>();
    vi.stubGlobal('fetch', vi.fn(() => res.promise));
    const { createAsr } = await import('./speechToText');
    const c = cb();
    const r = createAsr('en-US', 'push-to-talk', c);
    micDeferred.current.resolve(liveStream);
    await r.start();
    r.stop();
    await flush();
    r.abort();
    res.resolve(new Response(JSON.stringify({ text: 'too late' }), { status: 200 }));
    await flush(20);
    expect(c.onFinal).not.toHaveBeenCalled();
  });

  it('back-to-back pushes keep their audio separate', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ text: 'x' }), { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);
    const { createAsr } = await import('./speechToText');
    const c = cb();
    const r = createAsr('en-US', 'push-to-talk', c);
    micDeferred.current.resolve(liveStream);
    for (let i = 0; i < 2; i++) {
      await r.start();
      r.stop();
      await vi.waitFor(() => expect(r.active).toBe(false));
    }
    const sizes = (fetchMock.mock.calls as unknown as Call[]).map((x) => ((x[1].body as FormData).get('file') as Blob).size);
    expect(sizes).toEqual([5000, 5000]);
  });

  it('caps a single recording at MAX_RECORDING_MS (bounded buffer)', async () => {
    vi.useFakeTimers();
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ text: 'long' }), { status: 200 })));
    const { createAsr, MAX_RECORDING_MS } = await import('./speechToText');
    const c = cb();
    const r = createAsr('en-US', 'push-to-talk', c);
    micDeferred.current.resolve(liveStream);
    await r.start();
    await vi.advanceTimersByTimeAsync(MAX_RECORDING_MS + 10);
    expect(FakeRecorder.instances[0].state).toBe('inactive');
    await vi.waitFor(() => expect(c.onFinal).toHaveBeenCalled());
  });

  it('maps capture failures: denied → permission-denied (fatal), missing → capture-unavailable', async () => {
    const { createAsr } = await import('./speechToText');
    const c = cb();
    const r = createAsr('en-US', 'push-to-talk', c);
    micDeferred.current.reject(new DOMException('no', 'NotAllowedError'));
    await expect(r.start()).rejects.toBeTruthy();
    expect(c.onError).toHaveBeenCalledWith('permission-denied', expect.any(String), true);
    expect(r.active).toBe(false);

    micDeferred.current = deferred<MediaStream>();
    const c2 = cb();
    const r2 = createAsr('en-US', 'push-to-talk', c2);
    micDeferred.current.reject(new DOMException('none', 'NotFoundError'));
    await expect(r2.start()).rejects.toBeTruthy();
    expect(c2.onError).toHaveBeenCalledWith('capture-unavailable', expect.any(String), true);
  });

  it('surfaces transcription failures as non-fatal coded errors', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response('', { status: 502 })));
    const { createAsr } = await import('./speechToText');
    const c = cb();
    const r = createAsr('en-US', 'push-to-talk', c);
    micDeferred.current.resolve(liveStream);
    await r.start();
    r.stop();
    await vi.waitFor(() => expect(c.onError).toHaveBeenCalledWith('provider', 'Transcription failed (502).', false));
    await vi.waitFor(() => expect(r.active).toBe(false));
  });
});

describe('Web Speech adapter (streaming)', () => {
  class FakeSR {
    static all: FakeSR[] = [];
    lang = '';
    continuous = false;
    interimResults = false;
    maxAlternatives = 1;
    running = false;
    onresult: ((e: unknown) => void) | null = null;
    onerror: ((e: unknown) => void) | null = null;
    onend: (() => void) | null = null;
    onspeechstart: (() => void) | null = null;
    onspeechend: (() => void) | null = null;
    constructor() {
      FakeSR.all.push(this);
    }
    start() {
      this.running = true;
    }
    stop() {
      this.running = false;
      queueMicrotask(() => this.onend?.());
    }
    abort() {
      this.running = false;
    }
    emit(text: string, isFinal: boolean) {
      const result = Object.assign([{ transcript: text, confidence: 1 }], { isFinal });
      this.onresult?.({ resultIndex: 0, results: [result] });
    }
  }
  const last = () => FakeSR.all.at(-1)!;
  const running = () => FakeSR.all.some((r) => r.running);

  beforeEach(() => {
    FakeSR.all = [];
    (window as unknown as Record<string, unknown>).webkitSpeechRecognition = FakeSR;
  });

  it('P1: creating the adapter starts nothing; only start() starts the recognizer', async () => {
    const { createAsr } = await import('./speechToText');
    const r = createAsr('en-US', 'push-to-talk', cb());
    expect(r.engine).toBe('webspeech');
    expect(r.route).toBe('browser-speech');
    expect(FakeSR.all).toHaveLength(0);
    await r.start();
    expect(running()).toBe(true);
    expect(last().lang).toBe('en-US');
  });

  it('push-to-talk: accepts results until the tail ends, flushes, then stops for good (no restart)', async () => {
    vi.useFakeTimers();
    const { createAsr } = await import('./speechToText');
    const { RELEASE_TAIL_MS } = await import('./config');
    const c = cb();
    const r = createAsr('en-US', 'push-to-talk', c);
    await r.start();
    last().emit('hello', false);
    expect(c.onPartial).toHaveBeenLastCalledWith('hello');
    last().emit('hello there', true);
    r.stop();
    last().emit('tail word', true); // during the tail
    expect(running()).toBe(true);
    await vi.advanceTimersByTimeAsync(RELEASE_TAIL_MS + 1);
    expect(running()).toBe(false);
    expect(c.onEnd).toHaveBeenCalledTimes(1);
    expect(r.active).toBe(false);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(running()).toBe(false); // P1: no auto-restart after release
    expect(texts(c.onFinal)).toEqual(['hello there', 'tail word']);
  });

  it('a recognizer that never ends after stop() is forced down (FINALIZE_TIMEOUT_MS)', async () => {
    vi.useFakeTimers();
    const { createAsr } = await import('./speechToText');
    const { RELEASE_TAIL_MS, FINALIZE_TIMEOUT_MS } = await import('./config');
    const c = cb();
    const r = createAsr('en-US', 'push-to-talk', c);
    await r.start();
    last().stop = () => {}; // broken engine: ignores stop
    r.stop();
    await vi.advanceTimersByTimeAsync(RELEASE_TAIL_MS + FINALIZE_TIMEOUT_MS + 5);
    expect(r.active).toBe(false);
    expect(c.onEnd).toHaveBeenCalledTimes(1);
  });

  it('pressing again during the tail keeps the same recognizer running', async () => {
    vi.useFakeTimers();
    const { createAsr } = await import('./speechToText');
    const { RELEASE_TAIL_MS } = await import('./config');
    const r = createAsr('en-US', 'push-to-talk', cb());
    await r.start();
    r.stop();
    await r.start();
    await vi.advanceTimersByTimeAsync(RELEASE_TAIL_MS * 3);
    expect(running()).toBe(true);
    expect(FakeSR.all).toHaveLength(1);
  });

  it('restarts an engine that ends by itself while active; never after abort', async () => {
    vi.useFakeTimers();
    const { createAsr } = await import('./speechToText');
    const r = createAsr('en-US', 'continuous', cb());
    await r.start();
    last().running = false;
    last().onend!();
    await vi.advanceTimersByTimeAsync(200);
    expect(FakeSR.all).toHaveLength(2);
    expect(running()).toBe(true);
    r.abort();
    expect(running()).toBe(false);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(FakeSR.all).toHaveLength(2);
  });

  it('continuous: stop() flushes and ends without a tail', async () => {
    const { createAsr } = await import('./speechToText');
    const c = cb();
    const r = createAsr('en-US', 'continuous', c);
    await r.start();
    last().emit('one', true);
    r.stop();
    await flush(5);
    expect(c.onEnd).toHaveBeenCalled();
    expect(running()).toBe(false);
  });

  it('no results are delivered after abort', async () => {
    const { createAsr } = await import('./speechToText');
    const c = cb();
    const r = createAsr('en-US', 'push-to-talk', c);
    await r.start();
    const sr = last();
    r.abort();
    sr.emit('ghost', true);
    expect(c.onFinal).not.toHaveBeenCalled();
  });

  it('suppresses Chrome’s duplicate replay of the last final result', async () => {
    const { createAsr } = await import('./speechToText');
    const c = cb();
    const r = createAsr('en-US', 'push-to-talk', c);
    await r.start();
    last().emit('same', true);
    last().emit('same', true);
    expect(c.onFinal).toHaveBeenCalledTimes(1);
  });

  it('coded errors: permission → fatal + stopped; audio-capture → capture-unavailable; network non-fatal; routine silent', async () => {
    const { createAsr } = await import('./speechToText');
    const c = cb();
    const r = createAsr('en-US', 'push-to-talk', c);
    await r.start();
    last().onerror!({ error: 'no-speech' });
    expect(c.onError).not.toHaveBeenCalled();
    last().onerror!({ error: 'network' });
    expect(c.onError).toHaveBeenLastCalledWith('network', expect.any(String), false);
    last().onerror!({ error: 'not-allowed' });
    expect(c.onError).toHaveBeenLastCalledWith('permission-denied', expect.any(String), true);
    expect(r.active).toBe(false);
    expect(running()).toBe(false);

    const c2 = cb();
    const r2 = createAsr('en-US', 'push-to-talk', c2);
    await r2.start();
    last().onerror!({ error: 'audio-capture' });
    expect(c2.onError).toHaveBeenLastCalledWith('capture-unavailable', expect.any(String), true);
    expect(r2.active).toBe(false);
  });
});
