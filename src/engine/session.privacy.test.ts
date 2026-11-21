import { describe, it, expect } from 'vitest';
import { makeSession, connected, settle, FakeAsr, FIREFOX_LIKE } from '../test/engineFakes';
import { CONSENT_VERSION, disclosureFor } from './privacy';
import type { EngineEvent } from './events';

const capturing = (h: ReturnType<typeof makeSession>) => FakeAsr.all.some((a) => a.active);

describe('consent gate (P1)', () => {
  it('no recognizer is created or started before consent — not on connect, not on press', async () => {
    const h = makeSession();
    await connected(h);
    expect(h.session.getSnapshot().consent).toMatchObject({ granted: false, prompt: true, route: 'browser-speech' });
    h.session.pressTalk();
    await settle();
    expect(FakeAsr.all).toHaveLength(0);
    h.session.setListeningMode('continuous');
    await settle();
    expect(FakeAsr.all).toHaveLength(0);
    expect(h.session.getSnapshot().mode).toBe('push-to-talk');
  });

  it('declining keeps the call usable for listening and does not nag; pressing asks again', async () => {
    const h = makeSession();
    await connected(h);
    h.session.declineConsent();
    expect(h.session.getSnapshot().consent.prompt).toBe(false);
    h.call.receive({ t: 'segment', id: 's', original: 'hola', translated: 'hello', sourceLang: 'es-ES', targetLang: 'en-US', sentAt: 1 });
    expect(h.playback.played).toHaveLength(1);
    // Re-resolving features (e.g. after reconnect) must not re-prompt on its own.
    h.call.status('reconnecting');
    h.call.status('connected');
    await settle();
    expect(h.session.getSnapshot().consent.prompt).toBe(false);
    h.session.pressTalk();
    expect(h.session.getSnapshot().consent.prompt).toBe(true);
    expect(FakeAsr.all).toHaveLength(0);
  });

  it('consent is per processing route and versioned in storage', async () => {
    const h = makeSession();
    await connected(h);
    h.session.grantConsent();
    expect(h.storage.data.get(`talktual.consent.v${CONSENT_VERSION}.browser-speech`)).toBe('granted');
    expect(h.storage.data.has(`talktual.consent.v${CONSENT_VERSION}.server-stt`)).toBe(false);

    // A browser that would route audio to the server STT asks separately.
    const h2 = makeSession({ browser: FIREFOX_LIKE });
    h2.storage.setItem(`talktual.consent.v${CONSENT_VERSION}.browser-speech`, 'granted');
    await connected(h2);
    expect(h2.session.getSnapshot().consent).toMatchObject({ route: 'server-stt', granted: false, prompt: true });
  });

  it('works without storage (private mode): consent lasts for the session only', async () => {
    const throwing = {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('blocked');
      },
      removeItem: () => {
        throw new Error('blocked');
      },
    };
    const h = makeSession({ deps: { storage: throwing } });
    await connected(h);
    expect(h.session.getSnapshot().consent.granted).toBe(false);
    h.session.grantConsent();
    h.session.pressTalk();
    await settle();
    expect(capturing(h)).toBe(true);
  });

  it('disclosure names the actual remote processors for each route', () => {
    const web = disclosureFor('browser-speech', 'gemini');
    expect(web.points.join(' ')).toMatch(/browser’s speech-recognition service/);
    expect(web.points.join(' ')).toMatch(/only while you are talking/);
    expect(web.points.join(' ')).toMatch(/Google Gemini/);
    const srv = disclosureFor('server-stt', 'deepl');
    expect(srv.points.join(' ')).toMatch(/ElevenLabs/);
    expect(srv.points.join(' ')).toMatch(/DeepL/);
    expect(disclosureFor('browser-speech', null).points.join(' ')).toMatch(/translation service configured/);
  });
});

describe('push-to-talk semantics', () => {
  it('capture starts on press and fully stops after release (no background listening)', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    expect(capturing(h)).toBe(false);
    h.session.pressTalk();
    await settle();
    expect(capturing(h)).toBe(true);
    expect(h.session.getSnapshot()).toMatchObject({ holding: true, capturing: true, activity: 'listening' });
    h.session.releaseTalk();
    expect(h.session.getSnapshot().activity).toBe('processing');
    await settle();
    expect(capturing(h)).toBe(false);
    expect(h.session.getSnapshot()).toMatchObject({ holding: false, capturing: false, activity: 'ready' });
  });

  it('emits privacy.changed with capturing true/false around each push', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    const seen: boolean[] = [];
    h.session.events.on('privacy.changed', (e) => seen.push(e.capturing));
    h.session.pressTalk();
    await settle();
    h.session.releaseTalk();
    await settle();
    expect(seen).toContain(true);
    expect(seen.at(-1)).toBe(false);
  });

  it('cannot talk while not connected (no capture into an empty room)', async () => {
    const h = makeSession({ consent: true });
    h.session.start();
    h.call.status('waiting-for-peer');
    h.session.pressTalk();
    await settle();
    expect(FakeAsr.all).toHaveLength(0);
  });
});

describe('continuous (hands-free) mode', () => {
  it('requires consent and an explicit switch; shows capturing; is not persisted to the next call', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.session.setListeningMode('continuous');
    await settle();
    expect(h.session.getSnapshot()).toMatchObject({ mode: 'continuous', capturing: true, activity: 'listening' });
    expect(FakeAsr.all.at(-1)!.mode).toBe('continuous');
    // Finals are sent without holding anything.
    FakeAsr.all.at(-1)!.say('hands free words');
    await settle(40);
    expect(h.call.segments()).toHaveLength(1);

    const next = makeSession({ consent: true });
    expect(next.session.getSnapshot().mode).toBe('push-to-talk');
  });

  it('is half-duplex: pauses capture while incoming speech plays, resumes after', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.session.setListeningMode('continuous');
    await settle();
    h.call.receive({ t: 'segment', id: 'in1', original: 'hola', translated: 'hello', sourceLang: 'es-ES', targetLang: 'en-US', sentAt: 1 });
    await settle();
    expect(capturing(h)).toBe(false);
    expect(h.session.getSnapshot().activity).toBe('paused');
    h.playback.finish();
    await settle();
    expect(capturing(h)).toBe(true);
    expect(h.session.getSnapshot().activity).toBe('listening');
  });

  it('switching back to push-to-talk stops capture at once', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.session.setListeningMode('continuous');
    await settle();
    h.session.setListeningMode('push-to-talk');
    expect(capturing(h)).toBe(false);
    expect(h.session.getSnapshot().capturing).toBe(false);
  });
});

describe('stop guarantees', () => {
  const scenarios: Array<[string, (h: ReturnType<typeof makeSession>) => void]> = [
    ['mic off', (h) => h.session.setMicEnabled(false)],
    ['leave', (h) => h.session.leave()],
    ['peer left', (h) => h.call.status('peer-left')],
    ['peer away', (h) => h.call.status('peer-away')],
    ['connection lost', (h) => h.call.status('reconnecting')],
    ['dispose', (h) => h.session.dispose()],
    ['device removed', (h) => h.capture.loseDevice()],
  ];

  for (const mode of ['push-to-talk', 'continuous'] as const) {
    it.each(scenarios)(`${mode}: %s stops capture immediately`, async (_name, act) => {
      const h = makeSession({ consent: true });
      await connected(h);
      if (mode === 'continuous') h.session.setListeningMode('continuous');
      else h.session.pressTalk();
      await settle();
      expect(capturing(h)).toBe(true);
      act(h);
      expect(capturing(h)).toBe(false);
      await settle(40);
      expect(capturing(h)).toBe(false); // and it doesn't come back by itself
    });
  }

  it('mic off → stopped; mic on → ready again (no capture until pressed)', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.session.setMicEnabled(false);
    expect(h.session.getSnapshot().activity).toBe('stopped');
    expect(h.capture.releases).toBeGreaterThan(0);
    h.session.pressTalk();
    await settle();
    expect(FakeAsr.all).toHaveLength(0);
    h.session.setMicEnabled(true);
    expect(h.session.getSnapshot().activity).toBe('ready');
    expect(capturing(h)).toBe(false);
  });

  it('a phrase delivered by a recognizer after it was stopped is never sent (defense in depth)', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.session.pressTalk();
    await settle();
    const asr = FakeAsr.all.at(-1)!;
    h.session.setMicEnabled(false);
    // A misbehaving engine fires a late result after abort.
    asr.cb.onFinal({ text: 'late', lang: 'en-US', startedAt: 0, endedAt: 1, engine: 'webspeech' });
    asr.cb.onPartial('late partial');
    await settle(40);
    expect(h.call.segments()).toHaveLength(0);
    expect(h.session.getSnapshot().segments).toHaveLength(0);
    expect(h.session.getSnapshot().partial.local).toBe('');
    expect(h.session.getDiagnostics().counters.stale_asr_results_dropped).toBe(1);
  });
});

describe('failure handling', () => {
  it('permission denied: stopped, permission=denied, sticky notice; listening continues', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    const errors: EngineEvent[] = [];
    h.session.events.on('error', (e) => errors.push(e));
    FakeAsr.denyNext = true;
    h.session.pressTalk();
    await settle();
    const s = h.session.getSnapshot();
    expect(s).toMatchObject({ permission: 'denied', activity: 'stopped', capturing: false, holding: false });
    expect(s.notice?.sticky).toBe(true);
    expect(s.notice?.message).toMatch(/blocked/i);
    expect(errors.at(-1)).toMatchObject({ code: 'permission-denied', fatal: true });
    // Further presses don't hammer the permission prompt.
    h.session.pressTalk();
    await settle();
    expect(FakeAsr.all.length).toBe(1);
    // Incoming speech still plays.
    h.call.receive({ t: 'segment', id: 'z', original: 'hola', translated: 'hello', sourceLang: 'es-ES', targetLang: 'en-US', sentAt: 1 });
    expect(h.playback.played).toHaveLength(1);
  });

  it('device removed: stopped + mic shown off; turning the mic back on and pressing re-acquires', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.session.pressTalk();
    await settle();
    h.capture.loseDevice();
    expect(h.session.getSnapshot().notice?.code).toBe('device-lost');
    expect(h.session.getSnapshot().activity).toBe('stopped');
    h.session.releaseTalk();
    h.session.setMicEnabled(true); // the user's retry action
    expect(h.session.getSnapshot().activity).toBe('ready');
    expect(h.session.getSnapshot().notice).toBeNull();
    h.session.pressTalk();
    await settle();
    expect(capturing(h)).toBe(true);
  });

  it('permission denied → mic on re-checks the permission; if now granted the user can talk again', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    FakeAsr.denyNext = true;
    h.session.pressTalk();
    await settle();
    expect(h.session.getSnapshot().permission).toBe('denied');
    // User fixes it in site settings, then taps the mic button.
    h.capture.perm = 'granted';
    h.session.setMicEnabled(true);
    await settle();
    expect(h.session.getSnapshot()).toMatchObject({ permission: 'granted', activity: 'ready' });
    h.session.pressTalk();
    await settle();
    expect(capturing(h)).toBe(true);
  });

  it('permission still denied after retry: stays blocked (no prompt loop)', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    FakeAsr.denyNext = true;
    h.session.pressTalk();
    await settle();
    h.capture.perm = 'denied';
    h.session.setMicEnabled(true);
    await settle();
    expect(h.session.getSnapshot().permission).toBe('denied');
    h.session.pressTalk();
    await settle();
    expect(FakeAsr.all.at(-1)!.starts).toBe(1);
  });

  it('playback failure is recorded on the segment and counted, the call continues', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    const statuses: string[] = [];
    h.session.events.on('playback.status', (e) => statuses.push(e.status));
    h.call.receive({ t: 'segment', id: 'pf', original: 'hola', translated: 'hello', sourceLang: 'es-ES', targetLang: 'en-US', sentAt: 1 });
    h.playback.fail();
    expect(h.session.getSnapshot().segments.find((s) => s.id === 'pf')!.playback).toBe('failed');
    expect(statuses).toEqual(['queued', 'started', 'failed']);
    expect(h.session.getSnapshot()).toMatchObject({ phase: 'connected', playing: false });
    // Text is still available to read.
    expect(h.session.getSnapshot().segments.find((s) => s.id === 'pf')!.translated).toBe('hello');
  });

  it('provider timeout during translation: untranslated path, call continues', async () => {
    const h = makeSession({
      consent: true,
      translate: async (r) => ({ segmentId: r.segmentId, status: 'failed', text: r.text, error: { code: 'timeout', message: 't' }, attempts: 2, latencyMs: 8000 }),
    });
    await connected(h);
    h.session.pressTalk();
    await settle();
    FakeAsr.all.at(-1)!.say('slow provider');
    await settle(40);
    expect(h.call.segments()[0]).toMatchObject({ untranslated: true });
    expect(h.session.getSnapshot().phase).toBe('connected');
  });
});
