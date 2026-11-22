import { describe, it, expect, vi } from 'vitest';
import { makeSession, connected, settle, failTranslate, FIREFOX_LIKE, FakeAsr } from '../test/engineFakes';
import { SUPPORTED_LANGUAGES } from '../types';
import type { PeerMessage } from './events';

const seg = (over: Partial<Extract<PeerMessage, { t: 'segment' }>> = {}): PeerMessage => ({
  t: 'segment',
  id: 'p1',
  original: 'hola',
  translated: 'hello',
  sourceLang: 'es-ES',
  targetLang: 'en-US',
  sentAt: 1,
  ...over,
});

async function talk(h: ReturnType<typeof makeSession>, text: string) {
  h.session.pressTalk();
  await settle();
  h.asr()!.say(text);
  h.session.releaseTalk();
  await settle(40);
}

describe('outgoing pipeline (ported from Phase 1 CallRoom tests)', () => {
  it('announces itself, translates a finalized phrase into the peer language and sends it', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    expect(h.call.sent[0]).toEqual({ t: 'hello', lang: 'en-US', name: 'Ana' });
    await talk(h, 'hello');
    const out = h.call.segments();
    expect(out).toHaveLength(1);
    expect(out[0]).toMatchObject({ original: 'hello', translated: '[es] hello', sourceLang: 'en-US', targetLang: 'es-ES' });
    expect(out[0].untranslated).toBeUndefined();
  });

  it('DEF-C02: a failed translation is sent marked untranslated and the speaker is told', async () => {
    const h = makeSession({ consent: true, translate: failTranslate });
    await connected(h);
    await talk(h, 'hello');
    expect(h.call.segments()[0]).toMatchObject({ original: 'hello', translated: 'hello', untranslated: true });
    expect(h.session.getSnapshot().notice?.message).toMatch(/sent untranslated/i);
    expect(h.session.getSnapshot().segments[0]).toMatchObject({ translation: 'failed', untranslated: true });
  });

  it('rate limiting is explained to the speaker', async () => {
    const h = makeSession({
      consent: true,
      translate: async (r) => ({ segmentId: r.segmentId, status: 'failed', text: r.text, error: { code: 'rate-limited', message: '' }, attempts: 1, latencyMs: 1 }),
    });
    await connected(h);
    await talk(h, 'hello');
    expect(h.session.getSnapshot().notice?.message).toMatch(/too many requests/i);
  });
});

describe('incoming pipeline (ported)', () => {
  it('plays an incoming segment already translated for us', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.call.receive(seg());
    expect(h.playback.played).toEqual([{ segmentId: 'p1', text: 'hello', lang: 'en-US' }]);
    expect(h.session.getSnapshot().segments.at(-1)).toMatchObject({ id: 'p1', speaker: 'peer', translated: 'hello' });
  });

  it('re-translates a segment aimed at the wrong language; on failure plays the original in its own voice', async () => {
    const h = makeSession({ consent: true, translate: failTranslate });
    await connected(h);
    h.call.receive(seg({ id: 'p2', translated: 'salut', targetLang: 'fr-FR' }));
    await settle();
    expect(h.playback.played).toEqual([{ segmentId: 'p2', text: 'hola', lang: 'es-ES' }]);
    expect(h.session.getSnapshot().notice?.message).toMatch(/playing their original words/i);
  });

  it('a sender-marked untranslated segment is re-translated locally when possible', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.call.receive(seg({ id: 'p3', translated: 'hola', untranslated: true }));
    await settle();
    expect(h.playback.played).toEqual([{ segmentId: 'p3', text: '[en] hola', lang: 'en-US' }]);
  });

  it('same-language peer: the original is played without translating', async () => {
    const h = makeSession({ consent: true });
    await connected(h, 'en-GB');
    h.call.receive(seg({ id: 'p4', original: 'cheers', translated: 'cheers', sourceLang: 'en-GB', targetLang: 'en-GB' }));
    await settle();
    expect(h.translate).not.toHaveBeenCalled();
    expect(h.playback.played[0].text).toBe('cheers');
  });

  it('peer partials show as captions; cleared by the final segment', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.call.receive({ t: 'partial', text: 'ho', lang: 'es-ES' });
    expect(h.session.getSnapshot().partial.peer).toBe('ho');
    h.call.receive(seg());
    expect(h.session.getSnapshot().partial.peer).toBe('');
  });

  it('live captions are translated when the server offers it (debounced)', async () => {
    vi.useFakeTimers();
    try {
      const h = makeSession({ consent: true, server: { liveCaptions: true, translator: 'gemini' } });
      h.session.start();
      h.call.status('connected');
      h.call.receive({ t: 'hello', lang: 'es-ES', name: 'Luis' });
      await vi.advanceTimersByTimeAsync(0);
      h.call.receive({ t: 'partial', text: 'hola', lang: 'es-ES' });
      h.call.receive({ t: 'partial', text: 'hola amigo', lang: 'es-ES' });
      await vi.advanceTimersByTimeAsync(500);
      expect(h.session.getSnapshot().partial.peer).toBe('[en] hola amigo');
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('segment identity and ordering (M2.4)', () => {
  it('each outgoing segment carries a unique id and a monotonic seq; local record uses the same id', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    await talk(h, 'one');
    await talk(h, 'two');
    const [a, b] = h.call.segments();
    expect(a.id).not.toBe(b.id);
    expect([a.seq, b.seq]).toEqual([0, 1]);
    const ids = h.session.getSnapshot().segments.map((s) => s.id);
    expect(ids).toEqual([a.id, b.id]);
  });

  it('translations finish out of order but are sent in spoken order', async () => {
    const resolvers: Array<() => void> = [];
    const h = makeSession({
      consent: true,
      translate: (r) =>
        new Promise((res) =>
          resolvers.push(() => res({ segmentId: r.segmentId, status: 'translated', text: `T(${r.text})`, attempts: 1, latencyMs: 1 }))
        ),
    });
    await connected(h);
    h.session.pressTalk();
    await settle();
    h.asr()!.say('first');
    h.asr()!.say('second');
    await settle();
    // Serial chain: only the first is in flight.
    expect(resolvers).toHaveLength(1);
    resolvers[0]();
    await settle();
    resolvers[1]();
    await settle();
    expect(h.call.segments().map((s) => s.original)).toEqual(['first', 'second']);
  });

  it('a resent segment (same id, e.g. after reconnect) is spoken once', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.call.receive(seg({ id: 'dup' }));
    h.call.receive(seg({ id: 'dup' }));
    expect(h.playback.played).toHaveLength(1);
    expect(h.session.getDiagnostics().counters.duplicate_segments_dropped).toBe(1);
  });

  it('local segment appears immediately as pending, then translated', async () => {
    let release!: () => void;
    const h = makeSession({
      consent: true,
      translate: (r) => new Promise((res) => (release = () => res({ segmentId: r.segmentId, status: 'translated', text: 'hola', attempts: 1, latencyMs: 1 }))),
    });
    await connected(h);
    h.session.pressTalk();
    await settle();
    h.asr()!.say('hello');
    await settle();
    expect(h.session.getSnapshot().segments[0]).toMatchObject({ speaker: 'local', original: 'hello', translation: 'pending' });
    expect(h.session.getSnapshot().translating).toBe(1);
    release();
    await settle();
    expect(h.session.getSnapshot().segments[0]).toMatchObject({ translation: 'translated', translated: 'hola' });
    expect(h.session.getSnapshot().translating).toBe(0);
    expect(h.session.getSnapshot().lastLatencyMs).not.toBeNull();
  });

  it('transcript is bounded', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    for (let i = 0; i < 520; i++) h.call.receive(seg({ id: `m${i}` }));
    expect(h.session.getSnapshot().segments.length).toBe(500);
  });
});

describe('interruption (M2.4)', () => {
  it('pressing to talk while audio plays cuts it off (barge-in) and emits an interruption', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    const kinds: string[] = [];
    h.session.events.on('interruption', (e) => kinds.push(e.kind));
    h.call.receive(seg({ id: 'a' }));
    h.call.receive(seg({ id: 'b' }));
    expect(h.session.getSnapshot().playing).toBe(true);
    h.session.pressTalk();
    expect(h.playback.cancelled).toBe(2);
    expect(h.session.getSnapshot().playing).toBe(false);
    expect(kinds).toContain('barge-in');
    expect(h.gesture).toHaveBeenCalled();
  });

  it('playback status updates the segment record', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.call.receive(seg({ id: 'a' }));
    expect(h.session.getSnapshot().segments.at(-1)!.playback).toBe('started');
    h.playback.finish();
    expect(h.session.getSnapshot().segments.at(-1)!.playback).toBe('ended');
    expect(h.session.getSnapshot().playing).toBe(false);
  });
});

describe('capabilities (ported DEF-C11)', () => {
  it('tells the user up front when no speech recognition is available, and cannot talk', async () => {
    const h = makeSession({ consent: true, browser: FIREFOX_LIKE, server: { stt: false } });
    await connected(h);
    const s = h.session.getSnapshot();
    expect(s.features.canTalk).toBe(false);
    expect(s.notice?.message).toMatch(/speech recognition isn’t available/i);
    h.session.pressTalk();
    expect(FakeAsr.all).toHaveLength(0);
  });

  it('the server-STT path stays usable when the server has STT; hands-free is not offered there', async () => {
    const h = makeSession({ consent: true, browser: FIREFOX_LIKE, server: { stt: true } });
    await connected(h);
    const s = h.session.getSnapshot();
    expect(s.features).toMatchObject({ canTalk: true, stt: 'server-stt', route: 'server-stt', continuous: false });
    expect(s.notice).toBeNull();
    h.session.setListeningMode('continuous');
    expect(h.session.getSnapshot().mode).toBe('push-to-talk');
  });
});

describe('diagnostics privacy (M2.4)', () => {
  it('a full conversation leaves no transcript text, names or translations in diagnostics', async () => {
    const h = makeSession({ consent: true, translate: failTranslate });
    await connected(h);
    await talk(h, 'my secret sentence');
    h.call.receive(seg({ id: 'x1', original: 'su frase secreta', translated: 'their secret phrase' }));
    const dump = JSON.stringify(h.session.getDiagnostics());
    for (const leak of ['secret', 'sentence', 'frase', 'Ana', 'Luis', 'hola']) expect(dump).not.toContain(leak);
    // ...but the useful structure is there.
    expect(dump).toContain('seg_final');
    expect(dump).toContain('seg_translated');
    expect(dump).toContain('receive_to_playback_ms');
  });
});

it('supports all listed languages in the picker data (ported)', () => {
  expect(SUPPORTED_LANGUAGES.length).toBe(31);
  expect(new Set(SUPPORTED_LANGUAGES.map((l) => l.code)).size).toBe(31);
});

describe('communication-loop diagnostics (reliability task)', () => {
  it('counts promoted interims and turns that produced no text — no words recorded', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.session.pressTalk();
    await settle();
    h.asr()!.cb.onFinal({ text: 'where is the station', lang: 'en-US', startedAt: 0, endedAt: 1, engine: 'webspeech', promoted: true });
    h.session.releaseTalk();
    await settle(40);
    h.session.pressTalk();
    await settle();
    h.session.releaseTalk();
    await settle(40);
    const d = h.session.getDiagnostics();
    expect(d.counters).toMatchObject({ asr_interim_promoted: 1, asr_release_no_text: 1 });
    expect(JSON.stringify(d)).not.toContain('station');
    expect(h.call.segments()).toHaveLength(1); // the promoted phrase was translated and sent
  });
});
