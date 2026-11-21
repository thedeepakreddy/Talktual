import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

let fetchMock: ReturnType<typeof vi.fn>;
const ok = (text: string) => new Response(JSON.stringify({ text, provider: 'gemini' }), { status: 200 });

async function load() {
  vi.resetModules();
  return import('./translate');
}

beforeEach(() => {
  fetchMock = vi.fn();
  vi.stubGlobal('fetch', fetchMock);
});
afterEach(() => vi.useRealTimers());

describe('translateText', () => {
  it('returns the translation and caches it', async () => {
    fetchMock.mockResolvedValue(ok('hola'));
    const { translateText } = await load();
    expect(await translateText(' hello ', 'en-US', 'es-ES')).toBe('hola');
    expect(await translateText('hello', 'en', 'es')).toBe('hola');
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual({ text: 'hello', from: 'en', to: 'es' });
  });

  it('same language and empty text need no request', async () => {
    const { translateText } = await load();
    expect(await translateText('hi', 'en-US', 'en-GB')).toBe('hi');
    expect(await translateText('  ', 'en', 'es')).toBe('');
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('deduplicates concurrent identical requests', async () => {
    fetchMock.mockResolvedValue(ok('hola'));
    const { translateText } = await load();
    const [a, b] = await Promise.all([translateText('hello', 'en', 'es'), translateText('hello', 'en', 'es')]);
    expect([a, b]).toEqual(['hola', 'hola']);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});

describe('DEF-C02: failures are explicit and never cached', () => {
  it('HTTP error rejects with TranslationError instead of returning the source text', async () => {
    fetchMock.mockResolvedValue(new Response('{"error":"translation_failed"}', { status: 502 }));
    const { translateText, TranslationError } = await load();
    await expect(translateText('hello', 'en', 'es')).rejects.toBeInstanceOf(TranslationError);
  });

  it('a 200 carrying an error (old server contract) is still a failure', async () => {
    fetchMock.mockResolvedValue(
      new Response(JSON.stringify({ text: 'hello', provider: 'none', error: 'x' }), { status: 200 })
    );
    const { translateText, TranslationError } = await load();
    await expect(translateText('hello', 'en', 'es')).rejects.toBeInstanceOf(TranslationError);
  });

  it('malformed / non-JSON responses are failures', async () => {
    fetchMock.mockResolvedValueOnce(new Response('<html>', { status: 200 }));
    fetchMock.mockResolvedValueOnce(new Response('{"text":42}', { status: 200 }));
    const { translateText, TranslationError } = await load();
    await expect(translateText('a', 'en', 'es')).rejects.toBeInstanceOf(TranslationError);
    await expect(translateText('a', 'en', 'es')).rejects.toBeInstanceOf(TranslationError);
  });

  it('a failure does not poison the cache: the next call retries and succeeds', async () => {
    fetchMock.mockResolvedValueOnce(new Response('', { status: 502 })).mockResolvedValueOnce(ok('hola'));
    const { translateText } = await load();
    await expect(translateText('hello', 'en', 'es')).rejects.toThrow();
    expect(await translateText('hello', 'en', 'es')).toBe('hola');
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it('network errors reject with reason "network"', async () => {
    fetchMock.mockRejectedValue(new TypeError('Failed to fetch'));
    const { translateText } = await load();
    await expect(translateText('hello', 'en', 'es')).rejects.toMatchObject({ reason: 'network' });
  });

  it('a stalled request times out with reason "timeout" (bounded latency)', async () => {
    vi.useFakeTimers();
    fetchMock.mockImplementation(
      (_u: string, init: RequestInit) =>
        new Promise((_r, rej) => init.signal!.addEventListener('abort', () => rej(new DOMException('a', 'AbortError'))))
    );
    const { translateText, TRANSLATE_TIMEOUT_MS } = await load();
    const p = translateText('hello', 'en', 'es');
    const assertion = expect(p).rejects.toMatchObject({ name: 'TranslationError', reason: 'timeout' });
    await vi.advanceTimersByTimeAsync(TRANSLATE_TIMEOUT_MS + 1);
    await assertion;
  });

  it('a caller abort (superseded live caption) is an AbortError, not a TranslationError', async () => {
    fetchMock.mockImplementation(
      (_u: string, init: RequestInit) =>
        new Promise((_r, rej) => init.signal!.addEventListener('abort', () => rej(new DOMException('a', 'AbortError'))))
    );
    const { translateText, TranslationError } = await load();
    const c = new AbortController();
    const p = translateText('hello', 'en', 'es', c.signal);
    c.abort();
    const err = await p.catch((e) => e);
    expect(err).not.toBeInstanceOf(TranslationError);
    expect(err.name).toBe('AbortError');
  });
});

describe('getCapabilities', () => {
  it('degrades to the keyless defaults when the server is unreachable', async () => {
    fetchMock.mockRejectedValue(new TypeError('down'));
    const { getCapabilities } = await load();
    expect(await getCapabilities()).toEqual({ tts: false, stt: false, translator: 'mymemory', liveCaptions: false });
  });
});
