import { describe, it, expect, vi, beforeEach } from 'vitest';
import { flush } from '../test/fakes';

// The no-key server answers /api/tts with 503, which routes playback through
// speechSynthesis. This fake records what was spoken and lets the test decide
// when each utterance finishes.
class FakeSynth {
  spoken: { text: string; lang: string; utt: { onend: null | (() => void); onerror: null | (() => void) } }[] = [];
  cancelled = 0;
  speak(utt: { text: string; lang: string; onend: null | (() => void); onerror: null | (() => void) }) {
    this.spoken.push({ text: utt.text, lang: utt.lang, utt });
  }
  cancel() {
    this.cancelled++;
    const last = this.spoken.at(-1);
    last?.utt.onerror?.();
  }
  getVoices() {
    return [];
  }
  finishLast() {
    this.spoken.at(-1)?.utt.onend?.();
  }
}

let synth: FakeSynth;
let fetchMock: ReturnType<typeof vi.fn>;

async function load() {
  vi.resetModules();
  return import('./textToSpeech');
}

beforeEach(() => {
  synth = new FakeSynth();
  Object.defineProperty(window, 'speechSynthesis', { value: synth, configurable: true });
  (globalThis as unknown as { SpeechSynthesisUtterance: unknown }).SpeechSynthesisUtterance = class {
    lang = '';
    voice: unknown = null;
    onend: null | (() => void) = null;
    onerror: null | (() => void) = null;
    constructor(public text: string) {}
  };
  fetchMock = vi.fn(async () => new Response(JSON.stringify({ fallback: 'browser' }), { status: 503 }));
  vi.stubGlobal('fetch', fetchMock);
  vi.spyOn(console, 'warn').mockImplementation(() => {});
});

describe('enqueueSpeech ordering', () => {
  it('plays segments strictly one after another, in order', async () => {
    const tts = await load();
    const done = [
      tts.enqueueSpeech('one', 'es-ES'),
      tts.enqueueSpeech('two', 'es-ES'),
      tts.enqueueSpeech('three', 'es-ES'),
    ];
    await vi.waitFor(() => expect(synth.spoken.length).toBe(1));
    expect(synth.spoken[0].text).toBe('one');
    expect(tts.isSpeaking()).toBe(true);
    synth.finishLast();
    await vi.waitFor(() => expect(synth.spoken.length).toBe(2));
    expect(synth.spoken[1].text).toBe('two');
    synth.finishLast();
    await vi.waitFor(() => expect(synth.spoken.length).toBe(3));
    synth.finishLast();
    await Promise.all(done);
    expect(synth.spoken.map((s) => s.text)).toEqual(['one', 'two', 'three']);
    expect(synth.spoken.every((s) => s.lang === 'es-ES')).toBe(true);
    expect(tts.isSpeaking()).toBe(false);
  });

  it('ignores empty text', async () => {
    const tts = await load();
    await tts.enqueueSpeech('   ', 'en-US');
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

describe('DEF-C01: barge-in cancels queued playback', () => {
  it('stopSpeaking drops segments already queued behind the current one', async () => {
    const tts = await load();
    const ends: string[] = [];
    const p = ['A', 'B', 'C'].map((t) => tts.enqueueSpeech(t, 'fr-FR', { onEnd: () => ends.push(t) }));
    await vi.waitFor(() => expect(synth.spoken.length).toBe(1));

    tts.stopSpeaking();
    await Promise.all(p);
    await flush(50);

    // Before the fix, B and C were fetched and spoken after A was cut off.
    expect(synth.spoken.map((s) => s.text)).toEqual(['A']);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(tts.isSpeaking()).toBe(false);
    expect(ends.sort()).toEqual(['A', 'B', 'C']);
  });

  it('segments enqueued after a barge-in play normally and alone', async () => {
    const tts = await load();
    tts.enqueueSpeech('old-1', 'en-US');
    tts.enqueueSpeech('old-2', 'en-US');
    await vi.waitFor(() => expect(synth.spoken.length).toBe(1));
    tts.stopSpeaking();
    const fresh = tts.enqueueSpeech('new', 'en-US');
    await vi.waitFor(() => expect(synth.spoken.length).toBe(2));
    expect(synth.spoken[1].text).toBe('new');
    expect(tts.isSpeaking()).toBe(true);
    synth.finishLast();
    await fresh;
    await flush(50);
    expect(synth.spoken.map((s) => s.text)).toEqual(['old-1', 'new']);
    expect(tts.isSpeaking()).toBe(false);
  });

  it('aborts the in-flight TTS request on barge-in', async () => {
    let seenSignal: AbortSignal | undefined;
    fetchMock.mockImplementation(
      (_url: string, init: RequestInit) =>
        new Promise((_res, rej) => {
          seenSignal = init.signal!;
          init.signal!.addEventListener('abort', () => rej(new DOMException('aborted', 'AbortError')));
        })
    );
    const tts = await load();
    const p = tts.enqueueSpeech('hola', 'es-ES');
    await vi.waitFor(() => expect(seenSignal).toBeDefined());
    tts.stopSpeaking();
    await p;
    expect(seenSignal!.aborted).toBe(true);
    // An aborted request must not fall back to speaking the text anyway.
    expect(synth.spoken).toHaveLength(0);
  });
});

let gainCount = -1;
describe('streamed PCM playback', () => {
  it('decodes s16le across odd chunk boundaries and schedules gaplessly', async () => {
    const started: { at: number; samples: number[] }[] = [];
    let gainDisconnected = 0;
    class FakeCtx {
      state = 'running';
      currentTime = 0;
      destination = {};
      createGain() {
        return { connect() {}, disconnect: () => gainDisconnected++ };
      }
      createBuffer(_c: number, length: number, rate: number) {
        const data = new Float32Array(length);
        return { duration: length / rate, getChannelData: () => data, data };
      }
      createBufferSource() {
        const node = {
          buffer: null as null | { data: Float32Array },
          onended: null as null | (() => void),
          connect() {},
          start(at: number) {
            started.push({ at, samples: Array.from(node.buffer!.data) });
            setTimeout(() => node.onended?.(), 0);
          },
          stop() {},
        };
        return node;
      }
      async resume() {}
    }
    vi.stubGlobal('AudioContext', FakeCtx);
    // Samples 1, -2, 16384 split as [01] [00 FE] [FF 00 40]
    const chunks = [new Uint8Array([0x01]), new Uint8Array([0x00, 0xfe]), new Uint8Array([0xff, 0x00, 0x40])];
    fetchMock.mockImplementation(async () => {
      const body = new ReadableStream({
        start(c) {
          chunks.forEach((ch) => c.enqueue(ch));
          c.close();
        },
      });
      return new Response(body, { status: 200, headers: { 'X-Audio-Format': 'pcm', 'X-Sample-Rate': '4' } });
    });
    const tts = await load();
    await tts.enqueueSpeech('hola', 'es-ES');

    const all = started.flatMap((s) => s.samples);
    expect(all).toEqual([1 / 32768, -2 / 32768, 16384 / 32768]);
    // Each chunk starts where the previous one ended.
    for (let i = 1; i < started.length; i++) {
      const prev = started[i - 1];
      expect(started[i].at).toBeCloseTo(prev.at + prev.samples.length / 4);
    }
    expect(synth.spoken).toHaveLength(0);
    gainCount = gainDisconnected;
  });

  it('DEF-C10: releases the per-segment GainNode when the segment finishes', () => {
    // Set by the previous test after a complete PCM segment played.
    expect(gainCount).toBe(1);
  });
});
