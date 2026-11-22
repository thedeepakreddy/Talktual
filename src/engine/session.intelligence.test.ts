// Phase 5 communication intelligence, integrated in CommunicationSession.
import { describe, it, expect, vi, afterEach } from 'vitest';
import { makeSession, connected, settle } from '../test/engineFakes';
import { isPeerMessage } from './events';
import type { CallMessage } from '../lib/webrtc';
import type { TranslationRequest, TranslationResult } from './ports';

type Seg = Extract<CallMessage, { t: 'segment' }>;
const ok = (req: TranslationRequest, text = `ES(${req.text})`): TranslationResult => ({ segmentId: req.segmentId, status: 'translated', text, attempts: 1, latencyMs: 1 });

async function setup(opts: { aci?: boolean; conv?: boolean; capsPeer?: string[]; translate?: (r: TranslationRequest) => Promise<TranslationResult> } = {}) {
  const h = makeSession({
    consent: true,
    deps: { intelligence: opts.aci ?? true, ...(opts.conv ? { conversation: true } : {}) },
    translate: opts.translate ?? (async (r) => ok(r)),
  });
  await connected(h);
  h.call.receive({ t: 'hello', lang: 'es-ES', name: 'Bea', ...(opts.capsPeer ? { caps: opts.capsPeer } : { caps: ['aci1'] }) });
  await settle();
  return h;
}
async function talk(h: Awaited<ReturnType<typeof setup>>, text: string) {
  h.session.pressTalk();
  await settle();
  h.asr()!.say(text);
  h.session.releaseTalk();
  await settle(40);
}
const sent = (h: Awaited<ReturnType<typeof setup>>) => h.call.segments();
const peer = (over: Partial<Seg>): Seg => ({ t: 'segment', id: 'p1', original: 'hola', translated: 'hello', sourceLang: 'es-ES', targetLang: 'en-US', sentAt: 1, ...over });
const rec = (h: Awaited<ReturnType<typeof setup>>, id: string) => h.session.getSnapshot().segments.find((s) => s.id === id)!;

afterEach(() => vi.useRealTimers());

describe('wire + negotiation', () => {
  it('advertises aci1 only when on; assist codes are validated', async () => {
    const on = await setup();
    expect(on.call.sent.find((m) => m.t === 'hello')).toMatchObject({ caps: ['aci1'] });
    const off = await setup({ aci: false });
    expect(off.call.sent.find((m) => m.t === 'hello')).not.toHaveProperty('caps');
    expect(isPeerMessage({ ...peer({}), assist: ['sense:bank:river', 'protect:2'] })).toBe(true);
    expect(isPeerMessage({ ...peer({}), assist: ['<b>x</b>'] })).toBe(false);
    expect(isPeerMessage({ ...peer({}), assist: Array(9).fill('protect:1') })).toBe(false);
  });
});

describe('context-aware outbound translation', () => {
  it('protects names and pre-edits a sense only with evidence; original unchanged; assist sent to aci1 peers', async () => {
    const seen: string[] = [];
    const h = await setup({ translate: async (r) => (seen.push(r.text), ok(r)) });
    await talk(h, 'We went fishing by the river');
    await talk(h, 'Tell Rose to wait at the bank');
    expect(seen[1]).toBe('Tell QX0Q to wait at the riverbank');
    const s = sent(h)[1];
    expect(s.original).toBe('Tell Rose to wait at the bank'); // what was said
    expect(s.translated).toBe('ES(Tell Rose to wait at the riverbank)'); // names restored
    expect(s.assist).toEqual(['protect:1', 'sense:bank:river']);
    const r = rec(h, s.id);
    expect(r.assisted).toBe(true);
    expect(r.notes?.map((n) => n.text).join(' ')).toMatch(/Context-assisted.*riverbank/);
  });

  it('no assist codes to a peer without aci1', async () => {
    const h = await setup({ capsPeer: [] });
    await talk(h, 'Tell Rose hello');
    expect(sent(h)[0]).not.toHaveProperty('assist');
  });

  it('translation failure never leaks the MT input (placeholders) — the original is sent untranslated', async () => {
    const h = await setup({ translate: async (r) => ({ segmentId: r.segmentId, status: 'failed', text: r.text, error: { code: 'http', message: 'x' }, attempts: 1, latencyMs: 1 }) });
    await talk(h, 'Tell Rose to call me');
    expect(sent(h)[0]).toMatchObject({ original: 'Tell Rose to call me', translated: 'Tell Rose to call me', untranslated: true });
    expect(JSON.stringify(h.call.sent)).not.toMatch(/QX\d/);
  });

  it('malformed provider output (lost placeholder) → one context-free retry, no leak', async () => {
    const seen: string[] = [];
    const h = await setup({ translate: async (r) => (seen.push(r.text), ok(r, r.text.includes('QX') ? 'Dile a Rosa que llame' : 'Dile a Rose que llame')) });
    await talk(h, 'Tell Rose to call');
    expect(seen).toEqual(['Tell QX0Q to call', 'Tell Rose to call']);
    expect(sent(h)[0].translated).toBe('Dile a Rose que llame');
    expect(sent(h)[0]).not.toHaveProperty('assist');
    expect(h.session.getDiagnostics().counters.aci_restore_fallback).toBe(1);
  });

  it('off switch mid-session: context cleared, next phrase translated context-free', async () => {
    const seen: string[] = [];
    const h = await setup({ translate: async (r) => (seen.push(r.text), ok(r)) });
    await talk(h, 'We went fishing by the river');
    expect(h.session.getSnapshot().intelligence.context?.turns).toBe(1);
    h.session.setIntelligenceEnabled(false);
    expect(h.session.getSnapshot().intelligence.context).toMatchObject({ turns: 0, entities: [] });
    await talk(h, 'Tell Rose to wait at the bank');
    expect(seen[1]).toBe('Tell Rose to wait at the bank');
    expect(sent(h)[1]).not.toHaveProperty('assist');
  });

  it('assist pref off → faithful MT input but context still available for notes', async () => {
    const seen: string[] = [];
    const h = await setup({ translate: async (r) => (seen.push(r.text), ok(r)) });
    h.session.setIntelligencePrefs({ assist: false });
    await talk(h, 'Tell Rose to wait at the bank');
    expect(seen[0]).toBe('Tell Rose to wait at the bank');
  });

  it('works with the Phase 4 layer on (merge + assist) and keeps Phase 4 semantics', async () => {
    const seen: string[] = [];
    const h = await setup({ conv: true, capsPeer: ['conv1', 'aci1'], translate: async (r) => (seen.push(r.text), ok(r)) });
    await talk(h, 'Tell Rose to wait at the bank');
    expect(sent(h).filter((m) => m.state !== 'provisional')).toHaveLength(1);
    expect(sent(h).at(-1)).toMatchObject({ state: 'final', original: 'Tell Rose to wait at the bank' });
    expect(seen.at(-1)).toContain('QX0Q');
  });
});

describe('inbound: notes, signals, corrections, concise', () => {
  it('notes are shown, never spoken; signals recorded; cross-turn correction annotates the earlier line', async () => {
    const h = await setup();
    h.call.receive(peer({ id: 'a', original: 'Nos vemos en la Plaza Mayor a las cinco', translated: 'See you at Plaza Mayor at five' }));
    h.call.receive(peer({ id: 'b', original: '¿Llegas allí en metro?', translated: 'Are you getting there by metro?' }));
    h.call.receive(peer({ id: 'c', original: 'Perdón, no a las cinco, a las seis', translated: 'Sorry, not at five, at six' }));
    await settle();
    expect(rec(h, 'b').notes?.[0].text).toBe('“allí” → Plaza Mayor (mentioned earlier)');
    expect(rec(h, 'b').signals?.map((s) => s.category)).toContain('question');
    expect(rec(h, 'c').notes?.[0].text).toBe('Corrects earlier: cinco → seis');
    expect(rec(h, 'a').correctedBy).toBe('c');
    const spoken = [...h.playback.played, ...h.playback.queue].map((p) => p.text);
    expect(spoken).toEqual(['See you at Plaza Mayor at five', 'Are you getting there by metro?', 'Sorry, not at five, at six']);
  });

  it('concise mode speaks without fillers; transcript keeps the faithful text; faithful is the default', async () => {
    const h = await setup();
    h.call.receive(peer({ id: 'a', original: 'Pues el tren llega tarde', translated: 'Well, you know, the train is late' }));
    expect(h.playback.played.at(-1)!.text).toBe('Well, you know, the train is late');
    h.session.setIntelligencePrefs({ mode: 'concise' });
    h.call.receive(peer({ id: 'b', original: 'Pues el bus también', translated: 'Well, you know, the bus is too' }));
    await settle();
    expect([...h.playback.played, ...h.playback.queue].map((p) => p.text)).toContain('The bus is too');
    expect(rec(h, 'b').translated).toBe('Well, you know, the bus is too');
    expect(rec(h, 'b').notes?.some((n) => n.kind === 'concise')).toBe(true);
  });

  it('peer explains context assistance with assist codes (aci1 only)', async () => {
    const h = await setup();
    h.call.receive(peer({ id: 'a', original: 'Me senté en el banco', translated: 'I sat on the bench', assist: ['sense:banco:bench'] }));
    expect(rec(h, 'a').assisted).toBe(true);
    expect(rec(h, 'a').notes?.[0].text).toMatch(/Their app used context/);
  });
});

describe('suggestions', () => {
  it('misunderstanding → rephrase suggestion to me; dismissible; never repeated; nothing auto-sent', async () => {
    vi.useFakeTimers();
    const h = await setup();
    h.session.pressTalk();
    await vi.advanceTimersByTimeAsync(0);
    h.asr()!.say('The appointment is at the clinic on Elm Street');
    h.session.releaseTalk();
    await vi.advanceTimersByTimeAsync(50);
    const before = h.call.sent.length;
    h.call.receive(peer({ id: 'q', original: '¿Cómo? No entiendo', translated: "What? I don't understand" }));
    await vi.advanceTimersByTimeAsync(0);
    const sug = h.session.getSnapshot().intelligence.suggestions;
    expect(sug).toEqual([expect.objectContaining({ kind: 'rephrase' })]);
    expect(h.call.sent.length).toBe(before); // nothing sent on our behalf
    h.session.dismissSuggestion(sug[0].id);
    expect(h.session.getSnapshot().intelligence.suggestions).toEqual([]);
    h.call.receive(peer({ id: 'q2', original: 'No entiendo', translated: "I don't understand" }));
    await vi.advanceTimersByTimeAsync(0);
    expect(h.session.getSnapshot().intelligence.suggestions).toEqual([]); // same key: no loop
    h.session.dispose();
  });

  it('suggestions expire on their own (snapshot refreshes)', async () => {
    vi.useFakeTimers();
    const h = await setup();
    h.session.pressTalk();
    await vi.advanceTimersByTimeAsync(0);
    h.asr()!.say('Meet me at the bank');
    h.session.releaseTalk();
    await vi.advanceTimersByTimeAsync(50);
    expect(h.session.getSnapshot().intelligence.suggestions).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(46_000);
    expect(h.session.getSnapshot().intelligence.suggestions).toHaveLength(0);
    h.session.dispose();
  });
});

describe('privacy lifecycle', () => {
  it('context cleared on reset, decline, and session end; reconnect keeps it for the same call', async () => {
    const h = await setup();
    await talk(h, 'Tell Rose to meet us at Plaza Mayor');
    expect(h.session.getSnapshot().intelligence.context?.entities.map((e) => e.text)).toEqual(expect.arrayContaining(['Rose', 'Plaza Mayor']));
    h.call.status('reconnecting');
    h.call.status('connected');
    expect(h.session.getSnapshot().intelligence.context?.turns).toBe(1);
    h.session.resetIntelligenceContext();
    expect(h.session.getSnapshot().intelligence.context?.turns).toBe(0);
    await talk(h, 'Tell Rose hello');
    h.session.declineConsent();
    expect(h.session.getSnapshot().intelligence.context?.turns).toBe(0);
    await talk(h, 'Tell Rose hello');
    h.session.leave();
    expect(h.session.getSnapshot().intelligence.context?.turns).toBe(0);
  });

  it('user can remove a context item (not re-learned) and pin a term', async () => {
    const h = await setup();
    await talk(h, 'Tell Rose hello');
    const rose = h.session.getSnapshot().intelligence.context!.entities.find((e) => e.text === 'Rose')!;
    h.session.removeContextItem(rose.id);
    await talk(h, 'Tell Rose again');
    expect(h.session.getSnapshot().intelligence.context!.entities.some((e) => e.text === 'Rose')).toBe(false);
    h.session.pinContextTerm('Talktual Pro');
    expect(h.session.getSnapshot().intelligence.context!.entities.find((e) => e.text === 'Talktual Pro')?.source).toBe('explicit');
  });

  it('diagnostics hold categories/counts/timings only — no words, names or notes', async () => {
    const h = await setup();
    await talk(h, 'Tell Rose to wait at the bank, please');
    h.call.receive(peer({ id: 'x', original: '¿Dónde está la Calle de Alcalá?', translated: 'Where is Calle de Alcalá?' }));
    const dump = JSON.stringify(h.session.getDiagnostics());
    for (const w of ['Rose', 'bank', 'Alcalá', 'Calle', 'riverbank', 'please']) expect(dump).not.toContain(w);
    expect(dump).toContain('aci_signal');
    expect(h.session.getDiagnostics().counters).toMatchObject({ signal_question: 1 });
  });

  it('disclosure mentions drafts (Phase 4) and on-device assistance (Phase 5) only when those are on', async () => {
    const h = makeSession({ deps: { intelligence: true, conversation: true } });
    await connected(h);
    await settle();
    h.session.showDisclosure();
    const pts = h.session.getSnapshot().consent.disclosure!.points.join(' ');
    expect(pts).toMatch(/draft translation/);
    expect(pts).toMatch(/runs on your device/);
    const plain = makeSession({});
    await connected(plain);
    await settle();
    plain.session.showDisclosure();
    expect(plain.session.getSnapshot().consent.disclosure!.points.join(' ')).not.toMatch(/draft|your device/);
  });

  it('Phase 4 drafts can be turned off', async () => {
    const h = await setup({ conv: true, capsPeer: ['conv1'] });
    h.session.setDraftsEnabled(false);
    h.session.pressTalk();
    await settle();
    h.asr()!.say('i would like to book a');
    await settle(40);
    expect(sent(h).filter((m) => m.state === 'provisional')).toHaveLength(0);
    expect(h.session.getSnapshot().conversation.drafts).toBe(false);
  });
});
