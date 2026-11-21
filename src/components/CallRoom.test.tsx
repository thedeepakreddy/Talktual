import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, act, fireEvent } from '@testing-library/react';
import { SUPPORTED_LANGUAGES, findLanguage } from '../types';
import type { CallHandlers, CallMessage } from '../lib/webrtc';
import type { RecognizerCallbacks } from '../lib/speechToText';

// --- module mocks: everything that touches the network or devices ----------
const h = vi.hoisted(() => ({
  handlers: null as null | CallHandlers,
  sent: [] as unknown[],
  closed: 0,
  recognizer: null as null | RecognizerCallbacks,
  engine: 'webspeech' as 'webspeech' | 'scribe',
  caps: { tts: false, stt: false, translator: 'mymemory', liveCaptions: false },
  translate: null as null | ((t: string, f: string, to: string) => Promise<string>),
  spoken: [] as { text: string; lang: string }[],
  stopped: 0,
  released: 0,
}));

vi.mock('../lib/webrtc', () => ({
  connectCall: (_id: string, _url: string, handlers: CallHandlers) => {
    h.handlers = handlers;
    return { send: (m: unknown) => h.sent.push(m), close: () => h.closed++ };
  },
}));
vi.mock('../lib/speechToText', () => ({
  createRecognizer: (_lang: string, cb: RecognizerCallbacks) => {
    h.recognizer = cb;
    return { engine: h.engine, arm: async () => {}, hold: () => {}, release: () => {}, destroy: () => {} };
  },
}));
vi.mock('../lib/translate', async () => {
  const real = await vi.importActual<typeof import('../lib/translate')>('../lib/translate');
  return {
    TranslationError: real.TranslationError,
    translateText: (t: string, f: string, to: string) => h.translate!(t, f, to),
    getCapabilities: async () => h.caps,
    prewarm: () => {},
  };
});
vi.mock('../lib/textToSpeech', () => ({
  enqueueSpeech: async (text: string, lang: string) => {
    h.spoken.push({ text, lang });
  },
  stopSpeaking: () => h.stopped++,
  isSpeaking: () => false,
  preloadVoices: () => {},
}));
vi.mock('../lib/audio', () => ({ unlockAudio: async () => {}, releaseMic: () => h.released++ }));

const { CallRoom } = await import('./CallRoom');
const { TranslationError } = await import('../lib/translate');

const en = findLanguage('en')!;
const es = findLanguage('es')!;

beforeEach(() => {
  h.sent = [];
  h.closed = 0;
  h.spoken = [];
  h.stopped = 0;
  h.released = 0;
  h.engine = 'webspeech';
  h.caps = { tts: false, stt: false, translator: 'mymemory', liveCaptions: false };
  h.translate = async (t) => `[es] ${t}`;
});

function renderRoom(onEndCall = vi.fn()) {
  const utils = render(
    <CallRoom roomCode="ABC123" language={en} userName="Ana" onLanguageChange={() => {}} onEndCall={onEndCall} />
  );
  return { ...utils, onEndCall };
}

async function connect() {
  await act(async () => {
    h.handlers!.onStatus('connected');
    h.handlers!.onMessage({ t: 'hello', lang: es.bcp47, name: 'Luis' });
  });
}

const segments = () => h.sent.filter((m): m is Extract<CallMessage, { t: 'segment' }> => (m as CallMessage).t === 'segment');

describe('CallRoom', () => {
  it('announces itself, translates a finalized phrase into the peer language and sends it', async () => {
    renderRoom();
    expect(h.sent[0]).toEqual({ t: 'hello', lang: 'en-US', name: 'Ana' });
    await connect();
    await act(async () => h.recognizer!.onSegment('hello'));
    expect(segments()).toHaveLength(1);
    expect(segments()[0]).toMatchObject({ original: 'hello', translated: '[es] hello', sourceLang: 'en-US', targetLang: 'es-ES' });
    expect(segments()[0].untranslated).toBeUndefined();
  });

  it('DEF-C02: a failed translation is sent marked untranslated and the speaker is told', async () => {
    h.translate = async () => {
      throw new TranslationError('down', 'http');
    };
    renderRoom();
    await connect();
    await act(async () => h.recognizer!.onSegment('hello'));
    expect(segments()[0]).toMatchObject({ original: 'hello', translated: 'hello', untranslated: true });
    expect(screen.getByText(/sent untranslated/i)).toBeTruthy();
  });

  it('plays an incoming segment already translated for us', async () => {
    renderRoom();
    await connect();
    await act(async () =>
      h.handlers!.onMessage({ t: 'segment', id: 's1', original: 'hola', translated: 'hello', sourceLang: 'es-ES', targetLang: 'en-US', sentAt: 1 })
    );
    expect(h.spoken).toEqual([{ text: 'hello', lang: 'en-US' }]);
    expect(screen.getByText('hello')).toBeTruthy();
  });

  it('re-translates a segment aimed at the wrong language; on failure plays the original in its own voice', async () => {
    h.translate = async () => {
      throw new TranslationError('down', 'http');
    };
    renderRoom();
    await connect();
    await act(async () =>
      h.handlers!.onMessage({ t: 'segment', id: 's2', original: 'hola', translated: 'salut', sourceLang: 'es-ES', targetLang: 'fr-FR', sentAt: 1 })
    );
    expect(h.spoken).toEqual([{ text: 'hola', lang: 'es-ES' }]);
    expect(screen.getByText(/playing their original words/i)).toBeTruthy();
  });

  it('a sender-marked untranslated segment is re-translated locally when possible', async () => {
    h.translate = async (t) => `[en] ${t}`;
    renderRoom();
    await connect();
    await act(async () =>
      h.handlers!.onMessage({ t: 'segment', id: 's3', original: 'hola', translated: 'hola', sourceLang: 'es-ES', targetLang: 'en-US', sentAt: 1, untranslated: true })
    );
    expect(h.spoken).toEqual([{ text: '[en] hola', lang: 'en-US' }]);
  });

  it('DEF-C11: tells the user up front when no speech recognition is available, and disables talk', async () => {
    h.engine = 'scribe';
    h.caps = { ...h.caps, stt: false };
    renderRoom();
    await connect();
    expect(await screen.findByText(/speech recognition isn’t available/i)).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Push to talk' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('the scribe path stays usable when the server has STT', async () => {
    h.engine = 'scribe';
    h.caps = { ...h.caps, stt: true };
    renderRoom();
    await connect();
    await act(async () => {});
    expect(screen.queryByText(/speech recognition isn’t available/i)).toBeNull();
    expect((screen.getByRole('button', { name: 'Push to talk' }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('leaving tears down playback, mic and transport', async () => {
    const { unmount, onEndCall } = renderRoom();
    await connect();
    fireEvent.click(screen.getByRole('button', { name: 'Leave channel' }));
    expect(onEndCall).toHaveBeenCalled();
    unmount();
    expect(h.closed).toBe(1);
    expect(h.stopped).toBeGreaterThan(0);
    expect(h.released).toBe(1);
  });

  it('peer leaving ends the call after a short notice', async () => {
    vi.useFakeTimers();
    try {
      const { onEndCall } = renderRoom();
      await act(async () => h.handlers!.onStatus('peer-left'));
      expect(screen.getAllByText(/left the channel/i).length).toBeGreaterThan(0);
      await act(async () => vi.advanceTimersByTime(2000));
      expect(onEndCall).toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it('peer names and captions are rendered as text, never as HTML', async () => {
    renderRoom();
    await act(async () => {
      h.handlers!.onStatus('connected');
      h.handlers!.onMessage({ t: 'hello', lang: 'es-ES', name: '<img src=x onerror=alert(1)>' });
      h.handlers!.onMessage({ t: 'partial', text: '<b>bold</b>', lang: 'es-ES' });
    });
    expect(document.querySelector('img')).toBeNull();
    expect(document.querySelector('b')).toBeNull();
    expect(screen.getByText('<b>bold</b>', { exact: false })).toBeTruthy();
  });

  it('supports all listed languages in the picker data', () => {
    expect(SUPPORTED_LANGUAGES.length).toBe(31);
    expect(new Set(SUPPORTED_LANGUAGES.map((l) => l.code)).size).toBe(31);
  });
});
