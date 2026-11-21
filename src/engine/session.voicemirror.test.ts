import { describe, it, expect } from 'vitest';
import { makeSession, connected, settle, FakeVoiceTap, synthVoice, FakeAsr } from '../test/engineFakes';
import type { PeerMessage } from './events';

const DESC = { v: 1 as const, pitchStep: 22, pitchRange: 'normal' as const, tempoPct: 110 };
const seg = (id: string): PeerMessage => ({ t: 'segment', id, original: 'hola', translated: 'hello', sourceLang: 'es-ES', targetLang: 'en-US', sentAt: 1 });

function vm(opts: { consent?: boolean } = { consent: true }) {
  const tap = new FakeVoiceTap();
  const h = makeSession({ consent: opts.consent, deps: { voiceTap: tap } });
  return { ...h, tap };
}
const voiceMsgs = (h: ReturnType<typeof vm>) => h.call.sent.filter((m) => m.t === 'voice');

async function talkWithVoice(h: ReturnType<typeof vm>, seconds = 3) {
  h.session.pressTalk();
  await settle();
  h.tap.feed(synthVoice(190, seconds), 16000);
  FakeAsr.all.at(-1)!.say('hello there');
  h.session.releaseTalk();
  await settle(40);
}

describe('feature flag off (Phase 2 behaviour unchanged)', () => {
  it('not available; peer voice messages ignored; playback items carry no voice resolver', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    expect(h.session.getSnapshot().voice.available).toBe(false);
    h.session.toggleVoiceMirror();
    expect(h.session.getSnapshot().voice.prompt).toBe(false);
    h.call.receive({ t: 'voice', descriptor: DESC });
    expect(h.session.getDiagnostics().counters.voice_messages_ignored).toBe(1);
    h.call.receive(seg('a'));
    expect(h.playback.played[0].voice).toBeUndefined();
  });
});

describe('opt-in and capture gate', () => {
  it('off by default: talking never opens the analysis tap', async () => {
    const h = vm();
    await connected(h);
    expect(h.session.getSnapshot().voice).toMatchObject({ available: true, sharing: false });
    await talkWithVoice(h);
    expect(h.tap.opens).toBe(0);
    expect(voiceMsgs(h)).toHaveLength(0);
  });

  it('explicit opt-in: disclosure first; tap open only while talking; descriptor sent after enough speech', async () => {
    const h = vm();
    await connected(h);
    h.session.toggleVoiceMirror();
    expect(h.session.getSnapshot().voice.prompt).toBe(true);
    expect(h.session.getSnapshot().voice.sharing).toBe(false);
    h.session.acceptVoiceMirror();
    expect(h.session.getSnapshot().voice).toMatchObject({ prompt: false, sharing: true, descriptorSent: false });
    expect(h.tap.opens).toBe(0); // not talking yet
    h.session.pressTalk();
    await settle();
    expect(h.tap.open).toBe(true);
    h.tap.feed(synthVoice(190, 3), 16000);
    h.session.releaseTalk();
    await settle(40);
    expect(h.tap.open).toBe(false);
    const msgs = voiceMsgs(h);
    expect(msgs).toHaveLength(1);
    expect(msgs[0]).toMatchObject({ t: 'voice', descriptor: { v: 1 } });
    expect(h.session.getSnapshot().voice.descriptorSent).toBe(true);
  });

  it('declining the disclosure changes nothing', async () => {
    const h = vm();
    await connected(h);
    h.session.toggleVoiceMirror();
    h.session.declineVoiceMirror();
    await talkWithVoice(h);
    expect(h.tap.opens).toBe(0);
  });

  it('without microphone consent there is no capture, so no analysis either', async () => {
    const h = vm({ consent: false });
    await connected(h);
    h.session.declineConsent();
    h.session.toggleVoiceMirror();
    h.session.acceptVoiceMirror();
    h.session.pressTalk();
    await settle();
    expect(h.tap.opens).toBe(0);
    expect(h.session.getSnapshot().consent.prompt).toBe(true);
  });

  it('segment messages are unchanged: same ids/fields, no voice data inside segments', async () => {
    const h = vm();
    await connected(h);
    h.session.toggleVoiceMirror();
    h.session.acceptVoiceMirror();
    await talkWithVoice(h);
    const segs = h.call.segments();
    expect(segs).toHaveLength(1);
    expect(Object.keys(segs[0]).sort()).toEqual(['id', 'original', 'sentAt', 'seq', 'sourceLang', 't', 'targetLang', 'translated']);
  });
});

describe('listener side', () => {
  it('peer descriptor → hints applied when a segment starts; revoke applies to queued, not-yet-started segments', async () => {
    const h = vm();
    await connected(h);
    h.call.receive({ t: 'voice', descriptor: DESC });
    expect(h.session.getSnapshot().voice.peerActive).toBe(true);
    h.call.receive(seg('a')); // starts playing now
    h.call.receive(seg('b')); // queued
    const [a, b] = [h.playback.played[0], h.playback.queue[0]];
    expect(a.voice?.()).toMatchObject({ register: 'mid', ttsSpeed: 1.1 });
    h.call.receive({ t: 'voice', revoked: true });
    expect(h.session.getSnapshot().voice.peerActive).toBe(false);
    expect(b.voice?.()).toBeUndefined(); // resolved later → default voice
  });

  it('malformed / oversized voice messages never reach the controller', async () => {
    const h = vm();
    await connected(h);
    h.call.receive({ t: 'voice', descriptor: { ...DESC, pitchStep: 999 } } as unknown as PeerMessage);
    expect(h.session.getSnapshot().voice.peerActive).toBe(false);
  });
});

describe('lifecycle, revocation, cleanup', () => {
  it('revocation mid-call: tap closed, local data deleted, peer told to delete', async () => {
    const h = vm();
    await connected(h);
    h.session.toggleVoiceMirror();
    h.session.acceptVoiceMirror();
    await talkWithVoice(h);
    h.session.pressTalk();
    await settle();
    h.session.stopVoiceMirror();
    expect(h.tap.open).toBe(false);
    expect(voiceMsgs(h).at(-1)).toEqual({ t: 'voice', revoked: true });
    expect(h.session.getSnapshot().voice).toMatchObject({ sharing: false, descriptorSent: false });
    // toggling again starts with the disclosure, not silently
    h.session.toggleVoiceMirror();
    expect(h.session.getSnapshot().voice.prompt).toBe(true);
  });

  it.each([
    ['leave', (h: ReturnType<typeof vm>) => h.session.leave()],
    ['peer left', (h: ReturnType<typeof vm>) => h.call.status('peer-left')],
    ['dispose', (h: ReturnType<typeof vm>) => h.session.dispose()],
    ['connection failed', (h: ReturnType<typeof vm>) => h.call.status('disconnected')],
  ])('session end (%s): no voice data held, tap closed', async (_n, end) => {
    const h = vm();
    await connected(h);
    h.session.toggleVoiceMirror();
    h.session.acceptVoiceMirror();
    h.call.receive({ t: 'voice', descriptor: DESC });
    h.session.pressTalk();
    await settle();
    expect(h.session.holdsVoiceData()).toBe(true);
    end(h);
    expect(h.tap.open).toBe(false);
    expect(h.session.holdsVoiceData()).toBe(false);
  });

  it('mic off / peer away / reconnecting close the tap (same gate as capture)', async () => {
    for (const act of [
      (h: ReturnType<typeof vm>) => h.session.setMicEnabled(false),
      (h: ReturnType<typeof vm>) => h.call.status('peer-away'),
      (h: ReturnType<typeof vm>) => h.call.status('reconnecting'),
    ]) {
      const h = vm();
      await connected(h);
      h.session.toggleVoiceMirror();
      h.session.acceptVoiceMirror();
      h.session.pressTalk();
      await settle();
      expect(h.tap.open).toBe(true);
      act(h);
      expect(h.tap.open).toBe(false);
    }
  });

  it('reconnect: the current descriptor is re-sent (voice messages are not kept in the outbox)', async () => {
    const h = vm();
    await connected(h);
    h.session.toggleVoiceMirror();
    h.session.acceptVoiceMirror();
    await talkWithVoice(h);
    expect(voiceMsgs(h)).toHaveLength(1);
    h.call.status('reconnecting');
    h.call.status('connected');
    expect(voiceMsgs(h)).toHaveLength(2);
    expect(voiceMsgs(h)[1]).toEqual(voiceMsgs(h)[0]);
  });

  it('hands-free: opt-in while listening starts analysis; incoming playback pauses capture and the tap', async () => {
    const h = vm();
    await connected(h);
    h.session.setListeningMode('continuous');
    await settle();
    h.session.toggleVoiceMirror();
    h.session.acceptVoiceMirror();
    await settle();
    expect(h.tap.open).toBe(true);
    h.call.receive(seg('p'));
    await settle();
    expect(h.tap.open).toBe(false);
  });
});

describe('privacy of diagnostics', () => {
  it('no voice-derived numbers, descriptors or names in diagnostics — status enums only', async () => {
    const h = vm();
    await connected(h);
    h.session.toggleVoiceMirror();
    h.session.acceptVoiceMirror();
    await talkWithVoice(h);
    h.call.receive({ t: 'voice', descriptor: DESC });
    h.session.stopVoiceMirror();
    const d = h.session.getDiagnostics();
    const vmEvents = d.events.filter((e) => e.type === 'voicemirror');
    expect(vmEvents.length).toBeGreaterThan(3);
    for (const e of vmEvents) expect(Object.keys(e).sort()).toEqual(['status', 't', 'type']);
    const dump = JSON.stringify(d);
    // Voice data would appear as JSON keys (status names like "descriptor-sent" are fine;
    // a bare substring test is flaky because segment UUIDs are hex and can contain "f0").
    expect(dump).not.toMatch(/"(pitchStep|tempoPct|pitchRange|f0[A-Za-z]*|targetF0Hz|browserPitch|browserRate|ttsSpeed|register|descriptor|syllableRate|voicedSeconds)"\s*:/);
    for (const name of ['Ana', 'Luis']) expect(dump).not.toContain(name);
  });
});
