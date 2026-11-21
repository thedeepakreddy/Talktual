import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { deferred, flush } from '../test/fakes';

const micDeferred = { current: deferred<MediaStream>() };
vi.mock('./audio', () => ({ getMic: () => micDeferred.current.promise }));

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

const cb = () => ({ onSegment: vi.fn(), onPartial: vi.fn(), onSpeechActivity: vi.fn(), onError: vi.fn() });

beforeEach(() => {
  FakeRecorder.instances = [];
  micDeferred.current = deferred<MediaStream>();
  vi.stubGlobal('MediaRecorder', FakeRecorder);
  delete (window as unknown as Record<string, unknown>).webkitSpeechRecognition;
  delete (window as unknown as Record<string, unknown>).SpeechRecognition;
});
afterEach(() => vi.useRealTimers());

describe('Scribe (batch) recognizer', () => {
  it('records a push and posts it for transcription', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ text: ' hola ' }), { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);
    const { createRecognizer } = await import('./speechToText');
    const c = cb();
    const r = createRecognizer('es-ES', c);
    expect(r.engine).toBe('scribe');
    micDeferred.current.resolve(liveStream);
    await r.arm();
    r.hold();
    await flush();
    expect(FakeRecorder.instances.some((x) => x.state === 'recording')).toBe(true);
    r.release();
    await vi.waitFor(() => expect(c.onSegment).toHaveBeenCalledWith('hola'));
    const form = (fetchMock.mock.calls as unknown as [string, RequestInit][])[0][1].body as FormData;
    expect(form.get('language_code')).toBe('es');
  });

  it('DEF-C08: a tap released before the mic is ready never starts a runaway recording', async () => {
    vi.stubGlobal('fetch', vi.fn());
    const { createRecognizer } = await import('./speechToText');
    const r = createRecognizer('en-US', cb());
    void r.arm(); // mic prompt still open
    r.hold();
    r.release(); // quick tap
    micDeferred.current.resolve(liveStream);
    await flush(20);
    expect(FakeRecorder.instances.every((x) => x.state === 'inactive')).toBe(true);
  });

  it('DEF-C08: leaving the call mid-push does not upload the audio', async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    const { createRecognizer } = await import('./speechToText');
    const c = cb();
    const r = createRecognizer('en-US', c);
    micDeferred.current.resolve(liveStream);
    await r.arm();
    r.hold();
    await flush();
    r.destroy();
    await flush(20);
    expect(fetchMock).not.toHaveBeenCalled();
    expect(c.onSegment).not.toHaveBeenCalled();
  });

  it('back-to-back pushes keep their audio separate', async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ text: 'x' }), { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);
    const { createRecognizer } = await import('./speechToText');
    const r = createRecognizer('en-US', cb());
    micDeferred.current.resolve(liveStream);
    await r.arm();
    for (let i = 0; i < 2; i++) {
      r.hold();
      await flush();
      r.release();
    }
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    const sizes = (fetchMock.mock.calls as unknown as [string, RequestInit][]).map((c) => ((c[1].body as FormData).get('file') as Blob).size);
    expect(sizes).toEqual([5000, 5000]);
  });

  it('surfaces transcription failures as non-fatal errors', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response('', { status: 502 })));
    const { createRecognizer } = await import('./speechToText');
    const c = cb();
    const r = createRecognizer('en-US', c);
    micDeferred.current.resolve(liveStream);
    await r.arm();
    r.hold();
    await flush();
    r.release();
    await vi.waitFor(() => expect(c.onError).toHaveBeenCalledWith('Transcription failed (502).', false));
  });
});

describe('Web Speech (streaming) recognizer', () => {
  class FakeSR {
    static last: FakeSR;
    lang = '';
    continuous = false;
    interimResults = false;
    maxAlternatives = 1;
    started = 0;
    onresult: ((e: unknown) => void) | null = null;
    onerror: ((e: unknown) => void) | null = null;
    onend: (() => void) | null = null;
    onspeechstart: (() => void) | null = null;
    onspeechend: (() => void) | null = null;
    constructor() {
      FakeSR.last = this;
    }
    start() {
      this.started++;
    }
    stop() {
      this.onend?.();
    }
    abort() {}
    emit(text: string, isFinal: boolean) {
      const result = Object.assign([{ transcript: text, confidence: 1 }], { isFinal });
      this.onresult?.({ resultIndex: 0, results: [result] });
    }
  }

  beforeEach(() => {
    (window as unknown as Record<string, unknown>).webkitSpeechRecognition = FakeSR;
  });

  it('emits finalized phrases only while held (plus the release tail)', async () => {
    vi.useFakeTimers();
    const { createRecognizer } = await import('./speechToText');
    const { RELEASE_TAIL_MS } = await import('./config');
    const c = cb();
    const r = createRecognizer('en-US', c);
    expect(r.engine).toBe('webspeech');
    await r.arm();
    expect(FakeSR.last.lang).toBe('en-US');

    FakeSR.last.emit('not holding', true);
    expect(c.onSegment).not.toHaveBeenCalled();

    r.hold();
    FakeSR.last.emit('hello', false);
    expect(c.onPartial).toHaveBeenLastCalledWith('hello');
    FakeSR.last.emit('hello there', true);
    expect(c.onSegment).toHaveBeenCalledWith('hello there');
    r.release();
    FakeSR.last.emit('tail word', true);
    expect(c.onSegment).toHaveBeenCalledWith('tail word');
    vi.advanceTimersByTime(RELEASE_TAIL_MS + 1);
    FakeSR.last.emit('too late', true);
    expect(c.onSegment).toHaveBeenCalledTimes(2);
  });

  it('suppresses Chrome’s duplicate replay of the last final result', async () => {
    const { createRecognizer } = await import('./speechToText');
    const c = cb();
    const r = createRecognizer('en-US', c);
    await r.arm();
    r.hold();
    FakeSR.last.emit('same', true);
    FakeSR.last.emit('same', true);
    expect(c.onSegment).toHaveBeenCalledTimes(1);
  });

  it('permission denial is fatal; routine errors are silent', async () => {
    const { createRecognizer } = await import('./speechToText');
    const c = cb();
    const r = createRecognizer('en-US', c);
    await r.arm();
    FakeSR.last.onerror!({ error: 'no-speech' });
    expect(c.onError).not.toHaveBeenCalled();
    FakeSR.last.onerror!({ error: 'not-allowed' });
    expect(c.onError).toHaveBeenCalledWith(expect.stringContaining('permission'), true);
  });

  it('restarts after the engine ends on its own, but not after destroy', async () => {
    vi.useFakeTimers();
    const { createRecognizer } = await import('./speechToText');
    const r = createRecognizer('en-US', cb());
    await r.arm();
    const sr = FakeSR.last;
    expect(sr.started).toBe(1);
    sr.onend!();
    vi.advanceTimersByTime(200);
    expect(sr.started).toBe(2);
    r.destroy();
    sr.onend!();
    vi.advanceTimersByTime(1000);
    expect(sr.started).toBe(2);
  });
});
