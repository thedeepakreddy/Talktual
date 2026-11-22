import { describe, it, expect } from 'vitest';
import { IntelligenceController } from './controller';
import { extractEntities } from './extract';
import { SessionContext } from './context';
import { detectAmbiguity, detectIdioms } from './analyze';
import { detectSignals } from './signals';
import { buildMtInput, restore } from './protect';
import { concise, validateInbound, validateOutbound } from './policy';
import { AMBIGUOUS } from './lexicon';
import { redactSensitive } from './text';
import { DEFAULT_LIMITS, type OutboundDecision } from './types';

const mk = (now = () => 1000) => new IntelligenceController(now);
const out = (c: IntelligenceController, id: string, text: string) => c.outbound({ segmentId: id, text, lang: 'en-US', targetLang: 'es-ES' });
const inn = (c: IntelligenceController, id: string, original: string, translated = original) =>
  c.inbound({ segmentId: id, original, sourceLang: 'es-ES', translated, listenerLang: 'en-US' });
const cats = (s: { category: string; confidence: number }[], min = 0.5) => s.filter((x) => x.confidence >= min).map((x) => x.category).sort();
const noCtx = { previousByOther: false, previousWasClarification: false, topicWords: [], turnsSoFar: 0 };

describe('entity extraction', () => {
  it('names, places, times, dates; sentence-initial capitals are not names', () => {
    const e = extractEntities('Tomorrow my friend Rose meets us at Plaza Mayor at 5 pm', 'en');
    expect(e.map((x) => [x.text, x.kind])).toEqual(
      expect.arrayContaining([
        ['Rose', 'person'],
        ['Plaza Mayor', 'place'],
        ['5 pm', 'time'],
        ['tomorrow', 'date'],
      ])
    );
    expect(e.find((x) => x.text === 'Tomorrow')).toBeUndefined();
    expect(e.find((x) => x.text === 'Rose')?.protect).toBe(true);
    expect(e.find((x) => x.text === 'Plaza Mayor')?.protect).toBe(true); // address-like: keep verbatim
    expect(extractEntities('We fly to London on Friday', 'en').find((x) => x.text === 'London')).toMatchObject({ kind: 'place', protect: false }); // exonyms allowed
  });

  it('Spanish times and places', () => {
    const e = extractEntities('Nos vemos en la Calle de Alcalá a las cinco y media', 'es');
    expect(e.map((x) => x.text)).toEqual(expect.arrayContaining(['Calle de Alcalá', 'cinco y media']));
  });

  it('never extracts or stores sensitive identifiers', () => {
    const text = 'Call me on 612 345 678 or mail ana.lopez@example.com, card 4111 1111 1111 1111';
    expect(extractEntities(text, 'en').some((x) => /\d{3}|@/.test(x.text))).toBe(false);
    expect(redactSensitive(text).text).not.toMatch(/612|example\.com|4111/);
    const c = mk();
    out(c, 'a', text);
    expect(JSON.stringify(c.summary())).not.toMatch(/612|example|4111/);
  });
});

describe('bounded context', () => {
  it('evicts by turn count and character budget; reports observed maxima', () => {
    const ctx = new SessionContext({ ...DEFAULT_LIMITS, maxTurns: 3, maxTurnChars: 50 });
    for (let i = 0; i < 10; i++) ctx.addTurn(`s${i}`, 'local', `turn number ${i} with some words`, 'en', []);
    expect(ctx.size.turns).toBeLessThanOrEqual(3);
    expect(ctx.size.chars).toBeLessThanOrEqual(50);
    expect(ctx.maxObserved.turns).toBe(ctx.size.turns);
  });

  it('entities are LRU-bounded; pinned entities survive eviction; removal blocks re-learning', () => {
    const ctx = new SessionContext({ ...DEFAULT_LIMITS, maxEntities: 3 });
    const pinned = ctx.pin('Talktual Pro')!;
    for (const n of ['Ana', 'Bea', 'Carla', 'Dora']) ctx.addTurn(n, 'peer', `hi ${n}`, 'en', [{ text: n, kind: 'person', confidence: 0.7, protect: true }]);
    expect(ctx.size.entities).toBe(3);
    expect(ctx.entityList().some((e) => e.id === pinned.id)).toBe(true);
    const dora = ctx.entityList().find((e) => e.text === 'Dora')!;
    expect(ctx.remove(dora.id)).toBe(true);
    ctx.addTurn('x', 'peer', 'Dora again', 'en', [{ text: 'Dora', kind: 'person', confidence: 0.7, protect: true }]);
    expect(ctx.knows('Dora')).toBe(false);
    expect(ctx.pin('612 345 678 9')).toBeNull(); // no sensitive pins
    ctx.clear();
    expect(ctx.size).toEqual({ turns: 0, chars: 0, entities: 0 });
  });
});

describe('ambiguity', () => {
  it('resolves only with evidence; otherwise stays ambiguous', () => {
    expect(detectAmbiguity('I need cash from the bank', 'en', [])[0].sense?.id).toBe('finance');
    expect(detectAmbiguity('meet me at the bank', 'en', ['we went fishing on the river'])[0].sense?.id).toBe('river');
    expect(detectAmbiguity('meet me at the bank', 'en', [])[0].sense).toBeUndefined();
    // contradictory context → no guess
    expect(detectAmbiguity('meet me at the bank', 'en', ['fishing by the river', 'I need to deposit money'])[0].sense).toBeUndefined();
    expect(detectAmbiguity('me senté en el banco del parque', 'es', [])[0].sense?.id).toBe('bench');
  });

  it('a capitalized mid-sentence word is a name, not the ambiguous word', () => {
    expect(detectAmbiguity('I talked to Bill about the trip', 'en', []).map((f) => f.entry.word)).toEqual(['trip']);
    expect(detectAmbiguity('Change of plans: we leave at six', 'en', []).map((f) => f.entry.word)).toEqual(['change']);
  });

  it('idioms (longest match)', () => {
    expect(detectIdioms('Good luck, break a leg tonight!', 'en').map((i) => i.phrase)).toEqual(['break a leg']);
    expect(detectIdioms('That exam was a piece of cake', 'en').map((i) => i.phrase)).toEqual(['a piece of cake']);
    expect(detectIdioms('Me estás tomando el pelo', 'es')[0].meaning).toMatch(/teas/);
  });
});

describe('signals (categories + confidence only)', () => {
  it.each([
    ['Where is the station?', 'en', ['question']],
    ['Could you call a taxi for me?', 'en', ['request']],
    ['Shall I carry your bag?', 'en', ['offer']],
    ['Yes, that works for me', 'en', ['confirmation']],
    ["No, I can't make it", 'en', ['refusal']],
    ['No, I meant Tuesday', 'en', ['correction']],
    ['Sorry?', 'en', ['clarification-request']],
    ['Hurry, the train leaves right now', 'en', ['urgency']],
    ['By the way, did you book the hotel?', 'en', ['question', 'topic-change']],
    ['¿Dónde está el hotel?', 'es', ['question']],
    ['¿Me puedes ayudar con la maleta?', 'es', ['request']],
    ['Vale, de acuerdo', 'es', ['confirmation']],
    ['No puedo ir mañana', 'es', ['refusal']],
    ['Perdón, quise decir el jueves', 'es', ['correction']],
    ['Es urgente, llama a una ambulancia', 'es', ['request', 'urgency']],
  ])('%s → %j', (text, lang, want) => {
    expect(cats(detectSignals(text, lang as 'en', noCtx))).toEqual([...want].sort());
  });

  it('misunderstanding needs observable evidence; urgency only when explicit', () => {
    expect(cats(detectSignals('Sorry, what do you mean?', 'en', { ...noCtx, previousByOther: true }))).toContain('misunderstanding');
    expect(cats(detectSignals('Sorry, what do you mean?', 'en', noCtx))).not.toContain('urgency');
    expect(cats(detectSignals('I am so tired of waiting!!!', 'en', noCtx))).toEqual([]); // no emotion inference
    expect(cats(detectSignals('No entiendo', 'es', noCtx))).toContain('misunderstanding');
  });

  it('unsupported language → no signals rather than guesses', () => {
    expect(detectSignals('Wo ist der Bahnhof?', 'de', noCtx)).toEqual([]);
  });
});

describe('MT input protection', () => {
  it('builds deterministically and restores; malformed provider output is detected', () => {
    const prot = [{ token: 'QX0Q', value: 'Rose' }];
    const edits = [{ word: 'bank', senseId: 'river', replacement: 'riverbank' }];
    const mt = buildMtInput('Rose is at the bank', prot, edits);
    expect(mt).toBe('QX0Q is at the riverbank');
    expect(restore('QX0Q está en la orilla', prot)).toEqual({ text: 'Rose está en la orilla', ok: true });
    expect(restore('qx 0 q está en la orilla', prot).ok).toBe(true); // tolerant to provider spacing/case
    expect(restore('Rosa está en la orilla', prot).ok).toBe(false); // token lost
    expect(restore('QX0Q y QX7Q', prot).ok).toBe(false); // unknown token
  });
});

describe('validation (policy enforcement)', () => {
  const lex = AMBIGUOUS.en;
  const good = (): OutboundDecision => ({
    v: 1,
    type: 'outbound',
    segmentId: 's',
    mtInput: 'QX0Q is at the riverbank',
    protections: [{ token: 'QX0Q', value: 'Rose' }],
    edits: [{ word: 'bank', senseId: 'river', replacement: 'riverbank' }],
    assist: ['protect:1', 'sense:bank:river'],
    signals: [],
    notes: [],
    suggestions: [],
  });
  it('accepts a correct decision', () => expect(validateOutbound(good(), 'Rose is at the bank', lex, DEFAULT_LIMITS)).toBe(true));
  it.each([
    ['added content', (d: OutboundDecision) => ({ ...d, mtInput: d.mtInput + ' and she is happy' })],
    ['non-lexicon edit', (d: OutboundDecision) => ({ ...d, edits: [{ word: 'bank', senseId: 'river', replacement: 'beach' }], mtInput: 'QX0Q is at the beach' })],
    ['protecting text not spoken', (d: OutboundDecision) => ({ ...d, protections: [{ token: 'QX0Q', value: 'Paris' }] })],
    ['bad assist code', (d: OutboundDecision) => ({ ...d, assist: ['<script>'] })],
    ['wrong version', (d: OutboundDecision) => ({ ...d, v: 2 as never })],
    ['too many notes', (d: OutboundDecision) => ({ ...d, notes: Array(9).fill({ kind: 'idiom', text: 'x' }) })],
  ])('rejects %s', (_n, mutate) => {
    expect(validateOutbound(mutate(good()), 'Rose is at the bank', lex, DEFAULT_LIMITS)).toBe(false);
  });
  it('concise: closed-list fillers only; validator rejects reordering/additions', () => {
    expect(concise('Well, you know, the bus is late', 'en')).toEqual({ text: 'The bus is late', removed: 2 });
    expect(concise('The well is dry', 'en').removed).toBe(0); // "well" as a noun is kept
    expect(concise('O sea, pues llegamos tarde', 'es').text).toBe('Llegamos tarde');
    const base = { v: 1 as const, type: 'inbound' as const, segmentId: 's', notes: [], signals: [], suggestions: [] };
    expect(validateInbound({ ...base, speak: { text: 'the bus is late', removed: 2 } }, 'Well, you know, the bus is late', DEFAULT_LIMITS)).toBe(true);
    expect(validateInbound({ ...base, speak: { text: 'the bus is very late', removed: 2 } }, 'Well, you know, the bus is late', DEFAULT_LIMITS)).toBe(false);
    expect(validateInbound({ ...base, speak: { text: 'late', removed: 5 } }, 'Well, you know, the bus is late', DEFAULT_LIMITS)).toBe(false);
  });
});

describe('IntelligenceController', () => {
  it('context-assisted outbound: sense pre-edit + name protection; the original is untouched', () => {
    const c = mk();
    out(c, 'a', 'We went fishing by the river');
    // (A lone capitalized word at the START of a sentence is not treated as a name — documented limitation.)
    const d = out(c, 'b', 'Tell Rose to wait at the bank');
    expect(d.mtInput).toBe('Tell QX0Q to wait at the riverbank');
    expect(d.assist).toEqual(['protect:1', 'sense:bank:river']);
    expect(d.notes.map((n) => n.kind)).toEqual(['assisted', 'assisted']);
    expect(c.restoreTranslation(d, 'QX0Q esperará en la orilla').text).toBe('Rose esperará en la orilla');
  });

  it('assist off → faithful MT input; intelligence off → no analysis and context cleared', () => {
    const c = mk();
    c.setPrefs({ assist: false });
    out(c, 'a', 'We went fishing by the river');
    expect(out(c, 'b', 'Rose will wait at the bank').mtInput).toBe('Rose will wait at the bank');
    expect(c.summary().turns).toBe(2);
    c.setPrefs({ enabled: false });
    expect(c.summary()).toMatchObject({ turns: 0, entities: [] });
    expect(out(c, 'c', 'Rose at the bank').fallback).toBe('disabled');
    expect(c.summary().turns).toBe(0); // nothing learned while off
  });

  it('inbound notes: ambiguity, reference (single antecedent only), idiom, cross-turn correction', () => {
    const c = mk();
    out(c, 'a', 'Let us meet at Plaza Mayor at 5 pm');
    const ref = inn(c, 'b', '¿Nos vemos allí entonces?', 'Shall we meet there then?');
    expect(ref.notes.map((n) => n.text)).toContain('“allí” → Plaza Mayor (mentioned earlier)');
    const amb = inn(c, 'c', 'Llevo una vela', 'I am bringing a candle');
    expect(amb.notes[0]).toMatchObject({ kind: 'ambiguity' });
    const corr = out(c, 'd', 'Sorry, not 5 pm, 6 pm');
    expect(corr.correction).toMatchObject({ segmentId: 'a', from: '5 pm', to: '6 pm' });
    const id = inn(c, 'e', 'Bueno, mucha mierda', 'Well, break a leg');
    expect(id.notes).toEqual([]); // es idiom not in lexicon: no invented note
  });

  it('two candidate antecedents → no guess (unresolved-reference signal)', () => {
    const c = mk();
    inn(c, 'a', 'Mi amiga Lucía y mi amiga Marta vienen');
    const d = inn(c, 'b', '¿Ella trae el coche?');
    expect(d.notes.some((n) => n.kind === 'reference')).toBe(false);
    expect(cats(d.signals)).toContain('unresolved-reference');
  });

  it('a clear topic change breaks references', () => {
    const c = mk();
    out(c, 'a', 'The museum on Gran Vía Street was great');
    const d = inn(c, 'b', 'Cambiando de tema, ¿qué hay allí?');
    expect(d.notes.some((n) => n.kind === 'reference')).toBe(false);
  });

  it('suggestions: non-blocking, dismissible, no loops, superseded, dropped with their segment, expire', () => {
    let t = 0;
    const c = mk(() => t);
    out(c, 'a', 'Meet me at the bank'); // ambiguous, no evidence
    let s = c.activeSuggestions();
    expect(s).toHaveLength(1);
    expect(s[0]).toMatchObject({ kind: 'ambiguity', segmentId: 'a' });
    c.dismiss(s[0].id);
    out(c, 'b', 'The bank closes soon');
    expect(c.activeSuggestions()).toHaveLength(0); // same key never re-offered
    inn(c, 'c', '¿Cómo?'); // they didn't understand my last message
    s = c.activeSuggestions();
    expect(s).toEqual([expect.objectContaining({ kind: 'rephrase', segmentId: 'b' })]);
    c.dropSegment('b');
    expect(c.activeSuggestions()).toHaveLength(0);
    out(c, 'd', 'Bring the glasses');
    expect(c.activeSuggestions()).toHaveLength(1);
    t += DEFAULT_LIMITS.suggestionTtlMs + 1;
    expect(c.activeSuggestions()).toHaveLength(0);
    c.setPrefs({ suggestions: false });
    out(c, 'e', 'Pick up the fan');
    expect(c.activeSuggestions()).toHaveLength(0);
  });

  it('concise delivery is my preference, labelled, and the full text is kept by the caller', () => {
    const c = mk();
    c.setPrefs({ mode: 'concise' });
    const d = inn(c, 'a', 'Pues, o sea, el tren llega tarde', 'Well, you know, the train is late');
    expect(d.speak).toEqual({ text: 'The train is late', removed: 2 });
    expect(d.notes.at(-1)?.kind).toBe('concise');
    c.setPrefs({ mode: 'bogus' as never });
    expect(c.getPrefs().mode).toBe('faithful');
  });

  it('bounded processing: very long input falls back to faithful; per-decision time stays small', () => {
    const c = mk();
    expect(out(c, 'a', 'word '.repeat(200)).fallback).toBe('too-long');
    const t0 = performance.now();
    for (let i = 0; i < 200; i++) out(c, `s${i}`, `My friend Rose said the bank near Plaza Mayor opens at ${i % 12} pm, right?`);
    const per = (performance.now() - t0) / 200;
    expect(per).toBeLessThan(20); // generous CI bound; measured values are in the evaluation report
    expect(c.summary().turns).toBeLessThanOrEqual(DEFAULT_LIMITS.maxTurns);
    expect(c.summary().entities.length).toBeLessThanOrEqual(DEFAULT_LIMITS.maxEntities);
  });

  it('reset clears context, suggestions and the dismissal memory', () => {
    const c = mk();
    out(c, 'a', 'Meet me at the bank');
    c.reset();
    expect(c.summary().turns).toBe(0);
    expect(c.activeSuggestions()).toEqual([]);
  });
});
