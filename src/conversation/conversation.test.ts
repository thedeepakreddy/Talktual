import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { collapseRepetitions, isFillerOnly, looksIncomplete, resolveRestart } from './repair';
import { AdaptiveHold } from './pauseModel';
import { Segmenter, Unit } from './segmenter';
import { ListenerPolicy } from './listener';
import { LatencyMonitor, OutboundQueue } from './outbound';
import { canTransition, DEFAULT_CONVERSATION_CONFIG as CFG, UNIT_TRANSITIONS, UnitState } from './types';

describe('unit state model', () => {
  it('final is reachable only through turn-candidate; terminal states have no exits', () => {
    for (const [from, tos] of Object.entries(UNIT_TRANSITIONS)) {
      if (tos.includes('final')) expect(from).toBe('turn-candidate');
    }
    expect(UNIT_TRANSITIONS.superseded).toEqual([]);
    expect(UNIT_TRANSITIONS.interrupted).toEqual([]);
    expect(canTransition('final', 'provisional')).toBe(false); // committed text is never revised
    expect(canTransition('final', 'interrupted')).toBe(true);
  });
});

describe('completeness / repair heuristics', () => {
  it.each([
    ['i would like to book a', 'en', true],
    ['i think that', 'en', false], // 'that' often ends a sentence: not a hold cue
    ['how are you', 'en', false],
    ['and then the', 'en', true],
    ['table for two', 'en', false],
    ['good morning how are you', 'en', false],
    ['so I was going to,', 'en', true],
    ['I want to.', 'en', false],
    ['quiero reservar una', 'es', true],
    ['quiero reservar una mesa', 'es', false],
    ['je voudrais aller à', 'fr', true],
    ['the bus uh', 'en', true],
    ['some words to', 'ja', false], // unknown language: never hold on words
  ])('looksIncomplete(%j, %s) = %s', (t, l, want) => {
    expect(looksIncomplete(t, l)).toBe(want);
  });

  it('fillers', () => {
    expect(isFillerOnly('uh')).toBe(true);
    expect(isFillerOnly('Mm.')).toBe(true);
    expect(isFillerOnly('uh um')).toBe(true);
    expect(isFillerOnly('uh yes')).toBe(false);
    expect(isFillerOnly('no')).toBe(false);
  });

  it('collapses repeated phrases and function words, keeps deliberate content repeats', () => {
    expect(collapseRepetitions('can you can you call a taxi', 'en')).toEqual({ text: 'can you call a taxi', repaired: true });
    expect(collapseRepetitions('I want I want to go', 'en').text).toBe('I want to go');
    expect(collapseRepetitions('go to the the station', 'en').text).toBe('go to the station');
    expect(collapseRepetitions('it is very very good', 'en')).toEqual({ text: 'it is very very good', repaired: false });
    expect(collapseRepetitions('no no no no', 'en').text).toBe('no no no no');
    expect(collapseRepetitions('uh the bus', 'en').text).toBe('the bus');
    expect(collapseRepetitions('quiero quiero ir', 'es').text).toBe('quiero quiero ir'); // content word kept
  });

  it('detects restarts and corrections vs continuations', () => {
    expect(resolveRestart('i want to', 'i want to go to the airport', 'en')).toEqual({ kind: 'replace', text: 'i want to go to the airport' });
    expect(resolveRestart('we need the', 'we need a car', 'en')).toEqual({ kind: 'replace', text: 'we need a car' });
    expect(resolveRestart('i would like to book a', 'table for two', 'en')).toEqual({ kind: 'merge' });
    expect(resolveRestart('meet me at the', 'I mean, at the station', 'en')).toEqual({ kind: 'replace', text: 'at the station' });
    expect(resolveRestart('the train to', 'the bus is late', 'en')).toEqual({ kind: 'merge' }); // only first word shared
  });
});

describe('AdaptiveHold', () => {
  it('starts at the default and stays clamped', () => {
    const h = new AdaptiveHold(500, 2000, 900);
    expect(h.holdMs()).toBe(900);
    h.observeGap(100);
    h.observeGap(100);
    expect(h.holdMs()).toBe(500);
    for (let i = 0; i < 10; i++) h.observeGap(5000);
    expect(h.holdMs()).toBe(2000);
    h.observeGap(Number.NaN);
    h.observeGap(-1);
    h.reset();
    expect(h.holdMs()).toBe(900);
  });
});

describe('Segmenter', () => {
  let commits: Array<[Unit, string]>;
  let holds: Unit[];
  let revisions: Unit[];
  let discards: Unit[];
  let drops: string[];
  let illegal: number;
  let seg: Segmenter;
  let n: number;
  beforeEach(() => {
    vi.useFakeTimers();
    commits = [];
    holds = [];
    revisions = [];
    discards = [];
    drops = [];
    illegal = 0;
    n = 0;
    seg = new Segmenter(
      'en-US',
      CFG,
      {
        onHold: (u) => holds.push(u),
        onRevise: (u) => revisions.push(u),
        onCommit: (u, r) => commits.push([u, r]),
        onDiscard: (u) => discards.push(u),
        onDrop: (k) => drops.push(k),
        onIllegal: () => illegal++,
      },
      { now: () => Date.now(), setTimeout: (f, ms) => setTimeout(f, ms), clearTimeout: (t) => clearTimeout(t as number) },
      () => `u${n++}`
    );
    seg.beginTurn('T1');
  });
  afterEach(() => {
    expect(illegal).toBe(0);
    vi.useRealTimers();
  });

  it('commits complete-looking finals immediately (no added delay)', () => {
    seg.final('good morning how are you');
    expect(commits).toHaveLength(1);
    expect(commits[0][1]).toBe('complete');
    expect(commits[0][0]).toMatchObject({ id: 'u0', turn: 'T1', rev: 0 });
  });

  it('holds a hesitation and merges the continuation into ONE unit', () => {
    seg.final('i would like to book a');
    expect(commits).toHaveLength(0);
    expect(holds).toHaveLength(1);
    expect(seg.current?.state).toBe('pause-candidate');
    vi.advanceTimersByTime(600);
    seg.partial('table');
    vi.advanceTimersByTime(600); // > default hold since the final, but speech continued
    seg.final('table for two');
    expect(commits.map(([u]) => u.text)).toEqual(['i would like to book a table for two']);
  });

  it('does not keep a turn open forever: hold expires', () => {
    seg.final('i think about the');
    vi.advanceTimersByTime(CFG.defaultHoldMs - 1);
    expect(commits).toHaveLength(0);
    vi.advanceTimersByTime(1);
    expect(commits[0][1]).toBe('hold-expired');
  });

  it('continuous partials cannot extend a unit past maxOpenMs', () => {
    seg.final('and then the');
    for (let t = 0; t < CFG.maxOpenMs + 2000; t += 300) {
      seg.partial('still talking');
      vi.advanceTimersByTime(300);
    }
    expect(commits).toHaveLength(1);
    expect(commits[0][1]).toBe('max-open');
  });

  it('restart supersedes the held fragment: same id, new revision, one commit', () => {
    seg.final('i want to');
    seg.final('i want to go to the airport');
    expect(revisions).toHaveLength(1);
    expect(commits).toHaveLength(1);
    expect(commits[0][0]).toMatchObject({ id: 'u0', rev: 1, text: 'i want to go to the airport', repaired: true });
  });

  it('collapses repetition before commit and flags it', () => {
    seg.final('can you can you call a taxi');
    expect(commits[0][0]).toMatchObject({ text: 'can you call a taxi', repaired: true });
  });

  it('drops fillers and low-confidence short noise; flags uncertain speech', () => {
    seg.final('uh', 0.9);
    seg.final('mm');
    seg.final('yes', 0.2);
    seg.final('the bus leaves now', 0.4);
    seg.final('the next one', 0); // 0 = confidence unknown
    expect(drops).toEqual(['filler', 'filler', 'noise']);
    expect(commits.map(([u]) => [u.text, u.uncertain])).toEqual([
      ['the bus leaves now', true],
      ['the next one', false],
    ]);
  });

  it('push-to-talk release commits the held unit on recognizer end, or after the flush bound', () => {
    seg.final('i need to');
    seg.release();
    seg.endOfInput();
    expect(commits.map(([u, r]) => [u.text, r])).toEqual([['i need to', 'release']]);

    seg.beginTurn('T2');
    seg.final('and the');
    seg.release();
    vi.advanceTimersByTime(CFG.releaseFlushMs);
    expect(commits[1][1]).toBe('release');
  });

  it('a final after release commits straight away (merged with held text)', () => {
    seg.final('send it to');
    seg.release();
    seg.final('my office');
    expect(commits.map(([u]) => u.text)).toEqual(['send it to my office']);
  });

  it('caps unit size', () => {
    const long = Array.from({ length: 100 }, (_, i) => `w${i}x`).join(' ') + ' and'; // > 400 chars
    seg.final(long);
    expect(commits[0][1]).toBe('max-length');
  });

  it('cancel discards a held unit; nothing commits later; dispose clears learned timing', () => {
    seg.final('let me think about the');
    expect(seg.cancel()?.id).toBe('u0');
    vi.advanceTimersByTime(10_000);
    expect(commits).toHaveLength(0);
    expect(discards).toHaveLength(1);
    seg.dispose();
    expect(seg.current).toBeNull();
    expect(seg.holdMs).toBe(CFG.defaultHoldMs);
  });

  it('a new turn commits the previous held unit first', () => {
    seg.final('and so');
    seg.beginTurn('T2');
    expect(commits[0][0]).toMatchObject({ turn: 'T1', text: 'and so' });
    seg.final('hello there');
    expect(commits[1][0].turn).toBe('T2');
  });

  it('adapts the hold to the speaker but stays within bounds', () => {
    for (let i = 0; i < 4; i++) {
      seg.final('we go to the');
      vi.advanceTimersByTime(200);
      seg.final('park now');
      vi.advanceTimersByTime(1000);
    }
    expect(seg.holdMs).toBeGreaterThanOrEqual(CFG.minHoldMs);
    expect(seg.holdMs).toBeLessThan(CFG.defaultHoldMs);
  });

  it('every state reached is legal (exhaustive transition guard)', () => {
    const states = Object.keys(UNIT_TRANSITIONS) as UnitState[];
    expect(states).toContain('provisional');
    seg.final('the plan is to');
    seg.markProvisional('u0');
    expect(seg.current?.state).toBe('provisional');
    seg.final('the plan is to leave early'); // revise while provisional
    expect(commits).toHaveLength(1);
  });
});

describe('ListenerPolicy', () => {
  it('barge-in while the peer holds the floor interrupts that turn', () => {
    const p = new ListenerPolicy(4);
    p.onPeerFloor('T1', true, 0);
    expect(p.bargeIn(100, false)).toEqual(['T1']);
    expect(p.isInterrupted('T1')).toBe(true);
    expect(p.isInterrupted(undefined)).toBe(false);
  });

  it('a reply after the peer released is NOT an interruption', () => {
    const p = new ListenerPolicy(4);
    p.onPeerFloor('T1', true, 0);
    p.onPeerFloor('T1', false, 50);
    expect(p.bargeIn(100, false)).toEqual([]);
  });

  it('barge-in during playback interrupts the playing turns', () => {
    const p = new ListenerPolicy(4);
    p.noteActive('s1', 'T9');
    expect(p.bargeIn(0, true)).toEqual(['T9']);
    p.noteDone('s1');
  });

  it('a stale open floor (lost close message) expires', () => {
    const p = new ListenerPolicy(4, 12, 8000);
    p.onPeerFloor('T1', true, 0);
    expect(p.peerFloorOpen(7999)).toBe(true);
    expect(p.peerFloorOpen(8000)).toBe(false);
    p.touchPeerTurn('T1', 7000);
    expect(p.peerFloorOpen(14000)).toBe(true);
    p.resetFloor();
    expect(p.peerFloorOpen(14000)).toBe(false);
  });

  it('overlap hold is bounded and drops interrupted turns on release', () => {
    const p = new ListenerPolicy(4, 2);
    const item = (id: string) => ({ segmentId: id, text: 'x', lang: 'en' });
    expect(p.hold({ item: item('a'), turn: 'T1' })).toBeNull();
    expect(p.hold({ item: item('b'), turn: 'T2' })).toBeNull();
    expect(p.hold({ item: item('c'), turn: 'T2' })?.item.segmentId).toBe('a');
    p.onPeerFloor('T2', true, 0);
    p.bargeIn(1, false);
    expect(p.releaseHeld()).toEqual([]);
  });

  it('interrupted-turn memory is bounded', () => {
    const p = new ListenerPolicy(2);
    for (const t of ['A', 'B', 'C']) {
      p.onPeerFloor(t, true, 0);
      p.bargeIn(0, false);
    }
    expect(p.isInterrupted('A')).toBe(false);
    expect(p.isInterrupted('C')).toBe(true);
    p.clear();
    expect(p.isInterrupted('C')).toBe(false);
  });
});

describe('OutboundQueue / LatencyMonitor', () => {
  const u = (id: string, turn = 'T', text = 'some words'): Unit => ({ id, turn, text, rev: 0, repaired: false, uncertain: false });
  it('merges same-turn units behind an in-flight translation; bounded depth', () => {
    const q = new OutboundQueue(400);
    q.push(u('a'), 0);
    expect(q.take()?.unit.id).toBe('a');
    q.push(u('b'), 1);
    for (const id of ['c', 'd', 'e']) expect(q.push(u(id), 2).merged).toBe(true);
    expect(q.depth).toBe(2);
    q.done();
    const next = q.take()!;
    expect(next.unit.id).toBe('b');
    expect(next.absorbed).toEqual(['c', 'd', 'e']);
    expect(next.unit.text).toBe('some words some words some words some words');
  });

  it('does not merge across turns or past the size cap; does not merge when idle', () => {
    const q = new OutboundQueue(25);
    expect(q.push(u('a'), 0).merged).toBe(false);
    expect(q.push(u('b'), 0).merged).toBe(false); // nothing in flight yet
    q.take();
    q.push(u('c', 'T2'), 0);
    expect(q.push(u('d', 'T3'), 0).merged).toBe(false);
    expect(q.push(u('e', 'T3', 'x'.repeat(30)), 0).merged).toBe(false);
    q.clear();
    expect(q.depth).toBe(0);
  });

  it('degraded flag with hysteresis', () => {
    const m = new LatencyMonitor(2000);
    expect(m.observe(300)).toBe(false);
    m.observe(4000);
    m.observe(4000);
    expect(m.degraded).toBe(true);
    m.observe(1500);
    expect(m.degraded).toBe(true);
    for (let i = 0; i < 5; i++) m.observe(300);
    expect(m.degraded).toBe(false);
  });
});
