import { describe, it, expect, vi, beforeEach } from 'vitest';

// Real-world speech output: the older fakes always "spoke" and always ended,
// which hid the bugs below. This engine behaves like a real one — speak() can
// be refused (never starts), voices load late, and utterances report onstart.
class Engine {
  spoken: { text: string; lang: string; voice: unknown; u: Utt }[] = [];
  speaking = false;
  pending = false;
  paused = false;
  refuse = false; // e.g. iOS before any tap: speak() is silently ignored
  voices: Voice[] = [];
  private listeners: (() => void)[] = [];
  speak(u: Utt) {
    if (this.refuse) return;
    this.spoken.push({ text: u.text, lang: u.lang, voice: u.voice, u });
    this.speaking = true;
    u.onstart?.();
  }
  cancel() {
    this.speaking = false;
  }
  resume() {
    this.paused = false;
  }
  getVoices() {
    return this.voices;
  }
  addEventListener(_t: string, fn: () => void) {
    this.listeners.push(fn);
  }
  removeEventListener() {}
  loadVoices(v: Voice[]) {
    this.voices = v;
    this.listeners.forEach((f) => f());
  }
  finish() {
    this.speaking = false;
    this.spoken.at(-1)?.u.onend?.();
  }
}
interface Utt {
  text: string;
  lang: string;
  voice: unknown;
  volume?: number;
  onstart: null | (() => void);
  onend: null | (() => void);
  onerror: null | ((e: { error?: string }) => void);
}
interface Voice {
  name: string;
  lang: string;
  localService: boolean;
  default: boolean;
}
const v = (lang: string, name = lang, extra: Partial<Voice> = {}): Voice => ({ name, lang, localService: true, default: false, ...extra });

let engine: Engine;
let fetchMock: ReturnType<typeof vi.fn>;

async function load() {
  vi.resetModules();
  return import('./textToSpeech');
}

beforeEach(() => {
  engine = new Engine();
  Object.defineProperty(window, 'speechSynthesis', { value: engine, configurable: true });
  (globalThis as unknown as { SpeechSynthesisUtterance: unknown }).SpeechSynthesisUtterance = class {
    lang = '';
    voice: unknown = null;
    onstart = null;
    onend = null;
    onerror = null;
    constructor(public text: string) {}
  };
  fetchMock = vi.fn(async () => new Response('{"fallback":"browser"}', { status: 503 }));
  vi.stubGlobal('fetch', fetchMock);
  vi.spyOn(console, 'warn').mockImplementation(() => {});
});

function track(ctrl: { onStatus: (fn: (id: string, s: string, e?: string, f?: string) => void) => void }) {
  const log: string[] = [];
  ctrl.onStatus((id, s, e, f) => log.push(`${id}:${s}${e ? `:${e}` : ''}${f ? `:${f}` : ''}`));
  return log;
}

describe('voice selection by language', () => {
  it('prefers the exact locale, then the same language, never another language', async () => {
    const { pickVoice } = await load();
    const voices = [v('en-US', 'Samantha', { default: true }), v('es-MX', 'Paulina'), v('es-ES', 'Monica'), v('fr-FR', 'Thomas')];
    expect(pickVoice(voices, 'es-ES')?.name).toBe('Monica');
    expect(pickVoice(voices, 'es-AR')?.name).toMatch(/Paulina|Monica/);
    expect(pickVoice(voices, 'de-DE')).toBeNull();
  });

  it('understands Android-style tags (es_ES) and legacy codes (iw → he, in → id)', async () => {
    const { pickVoice } = await load();
    expect(pickVoice([v('en_US'), v('es_ES', 'android-es')], 'es-ES')?.name).toBe('android-es');
    expect(pickVoice([v('iw-IL', 'hebrew')], 'he-IL')?.name).toBe('hebrew');
    expect(pickVoice([v('in-ID', 'indo')], 'id-ID')?.name).toBe('indo');
  });

  it('on-device only: ignores network voices', async () => {
    const { pickVoice } = await load();
    expect(pickVoice([v('ja-JP', 'net', { localService: false })], 'ja-JP', true)).toBeNull();
  });

  it('speaks a translation with the voice of the target language', async () => {
    engine.voices = [v('en-US', 'Samantha', { default: true }), v('hi-IN', 'Lekha')];
    const { PlaybackController } = await load();
    const c = new PlaybackController();
    const p = c.enqueue({ segmentId: 'a', text: 'नमस्ते', lang: 'hi-IN' });
    await vi.waitFor(() => expect(engine.spoken).toHaveLength(1));
    expect(engine.spoken[0].lang).toBe('hi-IN');
    expect((engine.spoken[0].voice as Voice).name).toBe('Lekha');
    engine.finish();
    await p;
  });

  it('waits for a voice list that loads after the first request (Chrome)', async () => {
    const { PlaybackController } = await load();
    const c = new PlaybackController();
    const p = c.enqueue({ segmentId: 'a', text: 'bonjour', lang: 'fr-FR' });
    await new Promise((r) => setTimeout(r, 50));
    expect(engine.spoken).toHaveLength(0); // still waiting for voices
    engine.loadVoices([v('en-US'), v('fr-FR', 'Amelie')]);
    await vi.waitFor(() => expect(engine.spoken).toHaveLength(1));
    expect((engine.spoken[0].voice as Voice).name).toBe('Amelie');
    engine.finish();
    await p;
  });
});

describe('speech that never starts is reported, not faked', () => {
  it('a refused speak() (no tap yet) ends as failed:blocked, not ended', async () => {
    engine.voices = [v('es-ES')];
    engine.refuse = true;
    const { PlaybackController, SPEECH_START_TIMEOUT_MS } = await load();
    const c = new PlaybackController();
    const log = track(c);
    const t0 = Date.now();
    await c.enqueue({ segmentId: 'x', text: 'hola', lang: 'es-ES' });
    expect(log).toEqual(['x:queued', 'x:started', 'x:failed:blocked']);
    // Detected promptly instead of waiting out a silent clip.
    expect(Date.now() - t0).toBeLessThan(SPEECH_START_TIMEOUT_MS + 1500);
  });

  it('a not-allowed error is reported as blocked', async () => {
    engine.voices = [v('es-ES')];
    const { PlaybackController } = await load();
    const c = new PlaybackController();
    const log = track(c);
    const p = c.enqueue({ segmentId: 'x', text: 'hola', lang: 'es-ES' });
    await vi.waitFor(() => expect(engine.spoken).toHaveLength(1));
    engine.spoken[0].u.onerror?.({ error: 'not-allowed' });
    await p;
    expect(log.at(-1)).toBe('x:failed:blocked');
  });

  it('device-only with no voice for the language fails with no-voice', async () => {
    engine.voices = [v('en-US')];
    const { PlaybackController } = await load();
    const c = new PlaybackController();
    const log = track(c);
    await c.enqueue({ segmentId: 'x', text: 'こんにちは', lang: 'ja-JP', ttsRoute: () => 'device-only' });
    expect(log.at(-1)).toBe('x:failed:no-voice');
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('resumes a stuck paused engine before speaking', async () => {
    engine.voices = [v('en-US')];
    engine.paused = true;
    const { PlaybackController } = await load();
    const c = new PlaybackController();
    const p = c.enqueue({ segmentId: 'x', text: 'hello', lang: 'en-US' });
    await vi.waitFor(() => expect(engine.spoken).toHaveLength(1));
    expect(engine.paused).toBe(false);
    engine.finish();
    await p;
  });
});

describe('long text and the server', () => {
  it('splits long text into pieces spoken in order (Chrome ~15 s cutoff)', async () => {
    const { splitForSpeech, SPEECH_CHUNK_MAX } = await load();
    const text = Array.from({ length: 12 }, (_, i) => `This is sentence number ${i} of a long translation.`).join(' ');
    const parts = splitForSpeech(text);
    expect(parts.length).toBeGreaterThan(1);
    expect(parts.every((p) => p.length <= SPEECH_CHUNK_MAX)).toBe(true);
    expect(parts.join(' ')).toBe(text);
    const cjk = '翻'.repeat(400);
    expect(splitForSpeech(cjk).join('')).toBe(cjk);
  });

  it('a server without a TTS key is asked once, then the browser voice is used directly', async () => {
    engine.voices = [v('es-ES')];
    const { PlaybackController } = await load();
    const c = new PlaybackController();
    for (const id of ['a', 'b', 'c']) {
      const p = c.enqueue({ segmentId: id, text: id, lang: 'es-ES' });
      await vi.waitFor(() => expect(engine.speaking).toBe(true));
      engine.finish();
      await p;
    }
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(engine.spoken.map((s) => s.text)).toEqual(['a', 'b', 'c']);
  });

  it('a server that never answers falls back to the browser voice', async () => {
    engine.voices = [v('es-ES')];
    fetchMock.mockImplementation(
      (_u: string, init: RequestInit) => new Promise((_r, rej) => init.signal!.addEventListener('abort', () => rej(new DOMException('aborted', 'AbortError'))))
    );
    const { PlaybackController, SERVER_TTS_FIRST_BYTE_MS } = await load();
    const c = new PlaybackController();
    const log = track(c);
    const p = c.enqueue({ segmentId: 'slow', text: 'hola', lang: 'es-ES' });
    await vi.waitFor(() => expect(engine.spoken).toHaveLength(1), { timeout: SERVER_TTS_FIRST_BYTE_MS + 1500 });
    engine.finish();
    await p;
    expect(log.at(-1)).toBe('slow:ended:browser');
  }, 10_000);
});
