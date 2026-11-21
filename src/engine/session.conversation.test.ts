// Phase 4 conversation layer, integrated in CommunicationSession.
import { describe, it, expect, vi, afterEach } from 'vitest';
import { makeSession, connected, settle } from '../test/engineFakes';
import { isPeerMessage } from './events';
import type { CallMessage } from '../lib/webrtc';
import type { TranslationRequest, TranslationResult } from './ports';

type Seg = Extract<CallMessage, { t: 'segment' }>;
const CAPABLE_HELLO = { t: 'hello', lang: 'es-ES', name: 'Bea', caps: ['conv1'] } as const;
const OLD_HELLO = { t: 'hello', lang: 'es-ES', name: 'Bea' } as const;

async function setup(opts: { capable?: boolean; layer?: boolean; translate?: (r: TranslationRequest) => Promise<TranslationResult> } = {}) {
  const h = makeSession({ consent: true, deps: { conversation: opts.layer ?? true }, translate: opts.translate });
  await connected(h);
  h.call.receive(opts.capable === false ? OLD_HELLO : CAPABLE_HELLO);
  await settle();
  return h;
}
const finals = (h: Awaited<ReturnType<typeof setup>>) => h.call.segments().filter((m) => m.state !== 'provisional');
const provisionals = (h: Awaited<ReturnType<typeof setup>>) => h.call.segments().filter((m) => m.state === 'provisional');
const peerSeg = (over: Partial<Seg> = {}): Seg => ({
  t: 'segment',
  id: 'p1',
  original: 'hola',
  translated: 'hello',
  sourceLang: 'es-ES',
  targetLang: 'en-US',
  sentAt: 1,
  ...over,
});

afterEach(() => vi.useRealTimers());

describe('wire protocol (Phase 4 additions)', () => {
  it('validates the new messages and fields; rejects malformed ones', () => {
    expect(isPeerMessage({ t: 'hello', lang: 'en', name: 'A', caps: ['conv1'] })).toBe(true);
    expect(isPeerMessage({ t: 'hello', lang: 'en', name: 'A', caps: 'conv1' })).toBe(false);
    expect(isPeerMessage({ t: 'hello', lang: 'en', name: 'A', caps: ['x'.repeat(17)] })).toBe(false);
    expect(isPeerMessage({ t: 'floor', turn: 't1', open: true })).toBe(true);
    expect(isPeerMessage({ t: 'floor', turn: '', open: true })).toBe(false);
    expect(isPeerMessage({ t: 'floor', turn: 't1', open: 'yes' })).toBe(false);
    expect(isPeerMessage({ t: 'interrupt', turn: 't1' })).toBe(true);
    expect(isPeerMessage({ t: 'retract', id: 'x'.repeat(65) })).toBe(false);
    expect(isPeerMessage({ ...peerSeg(), state: 'provisional', rev: 2, turn: 't1', uncertain: true, repaired: false })).toBe(true);
    expect(isPeerMessage({ ...peerSeg(), state: 'maybe' })).toBe(false);
    expect(isPeerMessage({ ...peerSeg(), rev: -1 })).toBe(false);
    expect(isPeerMessage({ ...peerSeg(), rev: 1.5 })).toBe(false);
    expect(isPeerMessage({ t: 'partial', text: 'x', lang: 'en', turn: 7 })).toBe(false);
  });
});

describe('capability negotiation and rollback', () => {
  it('advertises conv1 only with the layer on', async () => {
    const on = await setup();
    expect(on.call.sent.find((m) => m.t === 'hello')).toMatchObject({ caps: ['conv1'] });
    expect(on.session.getSnapshot().conversation).toMatchObject({ enabled: true, peerCapable: true });
    const off = await setup({ layer: false });
    expect(off.call.sent.find((m) => m.t === 'hello')).not.toHaveProperty('caps');
    expect(off.session.getSnapshot().conversation.enabled).toBe(false);
  });

  it('a Phase 2/3 peer (no caps) never receives floor/provisional/interrupt messages or Phase 4 fields', async () => {
    const h = await setup({ capable: false });
    h.call.receive({ t: 'floor', turn: 'x', open: true }); // even if it somehow sent one
    h.session.pressTalk();
    await settle();
    h.asr()!.say('i would like to book a');
    await vi.waitFor(() => expect(h.session.getSnapshot().conversation.held).not.toBeNull());
    h.asr()!.say('table for two');
    h.session.releaseTalk();
    await settle(40);
    const kinds = new Set(h.call.sent.map((m) => m.t));
    expect(kinds.has('floor')).toBe(false);
    expect(kinds.has('interrupt')).toBe(false);
    expect(provisionals(h)).toHaveLength(0);
    expect(finals(h)).toHaveLength(1);
    expect(finals(h)[0]).not.toHaveProperty('state');
    expect(finals(h)[0]).not.toHaveProperty('turn');
    expect(finals(h)[0].original).toBe('i would like to book a table for two');
  });
});

describe('local speaker: turn/pause detection and self-repair', () => {
  it('hesitation: held, provisional sent to a capable peer, then ONE final with the same id', async () => {
    const h = await setup();
    h.session.pressTalk();
    await settle();
    expect(h.call.sent.filter((m) => m.t === 'floor')).toHaveLength(1);
    h.asr()!.say('i would like to book a');
    await settle(40);
    expect(finals(h)).toHaveLength(0);
    expect(h.session.getSnapshot().conversation.held).toMatchObject({ text: 'i would like to book a', provisional: true });
    const prov = provisionals(h);
    expect(prov).toHaveLength(1);
    h.asr()!.say('table for two');
    await settle(40);
    h.session.releaseTalk();
    await settle(40);
    expect(finals(h)).toHaveLength(1);
    expect(finals(h)[0]).toMatchObject({ id: prov[0].id, state: 'final', original: 'i would like to book a table for two' });
    expect(finals(h)[0].turn).toBe(prov[0].turn);
    expect(h.call.sent.filter((m) => m.t === 'floor').map((m) => (m as { open: boolean }).open)).toEqual([true, false]);
    expect(h.session.getSnapshot().conversation.held).toBeNull();
    // the provisional is never sent after its final
    expect(h.call.sent.indexOf(prov[0])).toBeLessThan(h.call.sent.indexOf(finals(h)[0]));
  });

  it('restart and repetition are repaired before anything is sent', async () => {
    const h = await setup({ capable: false });
    h.session.pressTalk();
    await settle();
    h.asr()!.say('i want to');
    h.asr()!.say('i want to go to the airport');
    h.asr()!.say('can you can you call a taxi');
    h.session.releaseTalk();
    await settle(40);
    expect(finals(h).map((m) => m.original)).toEqual(['i want to go to the airport', 'can you call a taxi']);
    expect(h.session.getSnapshot().segments.filter((s) => s.speaker === 'local').every((s) => s.repaired)).toBe(true);
  });

  it('fillers and low-confidence noise are not sent; low confidence is flagged uncertain', async () => {
    const h = await setup();
    h.session.pressTalk();
    await settle();
    const cb = h.asr()!.cb;
    const fin = (text: string, confidence: number) => cb.onFinal({ text, lang: 'en-US', startedAt: 0, endedAt: 1, engine: 'webspeech', confidence });
    fin('uh', 0.9);
    fin('ok', 0.1);
    fin('meet me outside', 0.4);
    h.session.releaseTalk();
    await settle(40);
    expect(finals(h).map((m) => [m.original, m.uncertain])).toEqual([['meet me outside', true]]);
    expect(h.session.getDiagnostics().counters).toMatchObject({ fillers_dropped: 1, noise_dropped: 1 });
  });

  it('a held fragment never stays open: it commits when the hold expires (hands-free)', async () => {
    vi.useFakeTimers();
    const h = await setup();
    h.session.setListeningMode('continuous');
    await vi.advanceTimersByTimeAsync(0);
    h.asr()!.say('send it to');
    await vi.advanceTimersByTimeAsync(100);
    expect(finals(h)).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(2500);
    expect(finals(h).map((m) => m.original)).toEqual(['send it to']);
    // floor closes after silence
    await vi.advanceTimersByTimeAsync(2000);
    expect(h.call.sent.filter((m) => m.t === 'floor').map((m) => (m as { open: boolean }).open)).toEqual([true, false]);
    h.session.dispose();
  });

  it('mic off while a provisional is out: the held unit is retracted and never finalized', async () => {
    const h = await setup();
    h.session.pressTalk();
    await settle();
    h.asr()!.say('my account number is in the');
    await settle(40);
    const [prov] = provisionals(h);
    expect(prov).toBeDefined();
    h.session.setMicEnabled(false);
    await settle(40);
    expect(h.call.sent).toContainEqual({ t: 'retract', id: prov.id });
    expect(finals(h)).toHaveLength(0);
  });
});

describe('listener: provisional/final, interruption, overlap', () => {
  it('provisional is shown, not spoken; final replaces it and plays once; stale revisions are ignored', async () => {
    const h = await setup();
    h.call.receive(peerSeg({ state: 'provisional', rev: 1, turn: 'T', translated: 'I would like' }));
    h.call.receive(peerSeg({ state: 'provisional', rev: 0, turn: 'T', translated: 'older' }));
    let rec = h.session.getSnapshot().segments.find((s) => s.id === 'p1')!;
    expect(rec).toMatchObject({ state: 'provisional', rev: 1, translated: 'I would like' });
    expect(h.playback.played).toHaveLength(0);
    h.call.receive(peerSeg({ state: 'final', rev: 1, turn: 'T', translated: 'I would like a table' }));
    h.call.receive(peerSeg({ state: 'final', rev: 1, turn: 'T', translated: 'I would like a table' })); // resend
    h.call.receive(peerSeg({ state: 'provisional', rev: 2, turn: 'T', translated: 'late provisional' }));
    await settle();
    rec = h.session.getSnapshot().segments.find((s) => s.id === 'p1')!;
    expect(rec).toMatchObject({ state: 'final', translated: 'I would like a table' });
    expect(h.playback.played.map((p) => p.text)).toEqual(['I would like a table']);
    expect(h.session.getSnapshot().segments.filter((s) => s.id === 'p1')).toHaveLength(1);
  });

  it('retract removes a provisional (but never a final)', async () => {
    const h = await setup();
    h.call.receive(peerSeg({ id: 'a', state: 'provisional', rev: 0 }));
    h.call.receive({ t: 'retract', id: 'a' });
    h.call.receive(peerSeg({ id: 'b' }));
    h.call.receive({ t: 'retract', id: 'b' });
    expect(h.session.getSnapshot().segments.map((s) => s.id)).toEqual(['b']);
  });

  it('barge-in while the peer holds the floor: interrupt sent; later segments of that turn are kept but not spoken', async () => {
    const h = await setup();
    h.call.receive({ t: 'floor', turn: 'T1', open: true });
    h.call.receive(peerSeg({ id: 's1', turn: 'T1', translated: 'first part' }));
    expect(h.playback.active).toBe(true);
    h.session.pressTalk();
    await settle();
    expect(h.call.sent).toContainEqual({ t: 'interrupt', turn: 'T1' });
    expect(h.playback.cancelled).toBe(1);
    h.session.releaseTalk();
    await settle(40);
    h.call.receive(peerSeg({ id: 's2', turn: 'T1', translated: 'stale continuation' }));
    await settle();
    expect(h.playback.played.map((p) => p.text)).toEqual(['first part']);
    const s2 = h.session.getSnapshot().segments.find((s) => s.id === 's2')!;
    expect(s2.interrupted).toBe(true);
    // A NEW turn from the peer plays normally.
    h.call.receive(peerSeg({ id: 's3', turn: 'T2', translated: 'new turn' }));
    expect(h.playback.played.map((p) => p.text)).toContain('new turn');
  });

  it('replying after the peer released is not an interruption; overlapping speech waits until we stop', async () => {
    const h = await setup();
    h.call.receive({ t: 'floor', turn: 'T1', open: true });
    h.call.receive({ t: 'floor', turn: 'T1', open: false });
    h.session.pressTalk();
    await settle();
    expect(h.call.sent.some((m) => m.t === 'interrupt')).toBe(false);
    h.call.receive(peerSeg({ id: 'late', turn: 'T1', translated: 'arrived while you talk' }));
    expect(h.playback.played).toHaveLength(0);
    expect(h.session.getSnapshot().conversation.overlapHeld).toBe(1);
    expect(h.session.getSnapshot().segments.find((s) => s.id === 'late')?.overlapHeld).toBe(true);
    h.session.releaseTalk();
    await settle();
    expect(h.playback.played.map((p) => p.text)).toEqual(['arrived while you talk']);
    expect(h.session.getSnapshot().conversation.overlapHeld).toBe(0);
    expect(h.playback.played[0].notAfter).toBeGreaterThan(0);
  });

  it('a lost floor-close does not make a later reply count as an interruption (reconnect resets the floor)', async () => {
    const h = await setup();
    h.call.receive({ t: 'floor', turn: 'T1', open: true });
    h.call.status('reconnecting');
    h.call.status('connected');
    h.session.pressTalk();
    await settle();
    expect(h.call.sent.some((m) => m.t === 'interrupt')).toBe(false);
  });

  it('when the peer interrupts us, our own turn is marked interrupted', async () => {
    const h = await setup();
    h.session.pressTalk();
    await settle();
    h.asr()!.say('let me explain everything');
    await settle(40);
    const turn = finals(h)[0].turn!;
    h.call.receive({ t: 'interrupt', turn });
    expect(h.session.getSnapshot().segments.find((s) => s.speaker === 'local')?.interrupted).toBe(true);
  });
});

describe('latency / load adaptation', () => {
  it('slow provider: same-turn phrases merge behind the in-flight translation; queue and calls stay bounded', async () => {
    const pending: Array<() => void> = [];
    const slow = (req: TranslationRequest) =>
      new Promise<TranslationResult>((resolve) =>
        pending.push(() => resolve({ segmentId: req.segmentId, status: 'translated', text: `T(${req.text})`, attempts: 1, latencyMs: 3000 }))
      );
    const h = await setup({ translate: slow, capable: false });
    h.session.pressTalk();
    await settle();
    for (let i = 0; i < 6; i++) h.asr()!.say(`phrase number ${i}`);
    await settle();
    expect(pending).toHaveLength(1); // one in flight; the rest wait as ONE merged unit
    expect(h.session.getDiagnostics().counters.units_merged).toBe(4);
    pending.shift()!();
    await settle(40);
    expect(pending).toHaveLength(1);
    pending.shift()!();
    await settle(40);
    h.session.releaseTalk();
    await settle(40);
    expect(finals(h).map((m) => m.original)).toEqual(['phrase number 0', 'phrase number 1 phrase number 2 phrase number 3 phrase number 4 phrase number 5']);
    // The merged phrases are one transcript record, not five.
    expect(h.session.getSnapshot().segments.filter((s) => s.speaker === 'local')).toHaveLength(2);
    expect(h.session.getSnapshot().translating).toBe(0);
  });

  it('sustained slow translations set degraded and stop provisional translations', async () => {
    vi.useFakeTimers();
    const slow = (req: TranslationRequest) =>
      new Promise<TranslationResult>((resolve) =>
        setTimeout(() => resolve({ segmentId: req.segmentId, status: 'translated', text: req.text, attempts: 1, latencyMs: 0 }), 3000)
      );
    const h = await setup({ translate: slow });
    h.session.pressTalk();
    await vi.advanceTimersByTimeAsync(0);
    h.asr()!.say('first sentence here');
    await vi.advanceTimersByTimeAsync(3100);
    h.asr()!.say('second sentence here');
    await vi.advanceTimersByTimeAsync(3100);
    expect(h.session.getSnapshot().conversation.degraded).toBe(true);
    const before = provisionals(h).length;
    h.asr()!.say('and then we go to the');
    await vi.advanceTimersByTimeAsync(4000);
    expect(provisionals(h).length).toBe(before);
    h.session.dispose();
  });
});

describe('context lifetime and privacy', () => {
  it('leave clears every piece of conversation context; nothing is sent afterwards', async () => {
    const h = await setup();
    h.call.receive({ t: 'floor', turn: 'T1', open: true });
    h.session.pressTalk();
    await settle();
    h.asr()!.say('we should go to the');
    await settle(40);
    h.session.leave();
    const n = h.call.sent.length;
    await settle(40);
    expect(h.session.getSnapshot().conversation).toMatchObject({ held: null, overlapHeld: 0, peerTalking: false });
    expect(h.call.sent.length).toBe(n);
    expect(finals(h)).toHaveLength(0);
  });

  it('diagnostics carry ids, enums and timings — no transcript words', async () => {
    const h = await setup();
    h.session.pressTalk();
    await settle();
    h.asr()!.say('my secret plan is to');
    h.asr()!.say('my secret plan is to leave early');
    h.session.releaseTalk();
    await settle(40);
    h.call.receive(peerSeg({ state: 'provisional', rev: 0, original: 'mi plan secreto', translated: 'my hidden plan' }));
    const dump = JSON.stringify(h.session.getDiagnostics()).toLowerCase();
    for (const w of ['secret', 'leave early', 'hidden', 'plan secreto']) expect(dump).not.toContain(w);
    expect(dump).toContain('conv_unit');
  });

  it('works with VoiceMirror absent (default): no voice messages are sent', async () => {
    const h = await setup();
    h.session.pressTalk();
    await settle();
    h.asr()!.say('hello there');
    h.session.releaseTalk();
    await settle(40);
    expect(h.session.getSnapshot().voice.available).toBe(false);
    expect(h.call.sent.some((m) => m.t === 'voice')).toBe(false);
    expect(finals(h)).toHaveLength(1);
  });
});
