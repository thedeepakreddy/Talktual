import { describe, it, expect, vi } from 'vitest';
import { ACTIVITY_TRANSITIONS, PHASE_TRANSITIONS, StateMachine, TERMINAL_PHASES } from './sessionMachine';
import { EventBus } from './eventBus';
import { makeEvent, ENGINE_EVENT_VERSION, isPeerMessage } from './events';
import { Diagnostics, redact } from './diagnostics';
import { BoundedSet, SegmentLedger } from './segments';
import type { Activity, CallPhase } from './events';

describe('state machines', () => {
  const phases = Object.keys(PHASE_TRANSITIONS) as CallPhase[];
  const activities = Object.keys(ACTIVITY_TRANSITIONS) as Activity[];

  it('phase: every (from, to) pair is allowed exactly when the table says so', () => {
    for (const from of phases) {
      for (const to of phases) {
        if (from === to) continue;
        const onIllegal = vi.fn();
        const m = new StateMachine<CallPhase>(from, PHASE_TRANSITIONS, () => {}, onIllegal);
        const ok = m.to(to);
        expect(ok, `${from} → ${to}`).toBe(PHASE_TRANSITIONS[from].includes(to));
        expect(m.state).toBe(ok ? to : from);
        expect(onIllegal).toHaveBeenCalledTimes(ok ? 0 : 1);
      }
    }
  });

  it('activity: every (from, to) pair is allowed exactly when the table says so', () => {
    for (const from of activities) {
      for (const to of activities) {
        if (from === to) continue;
        const m = new StateMachine<Activity>(from, ACTIVITY_TRANSITIONS, () => {});
        expect(m.to(to), `${from} → ${to}`).toBe(ACTIVITY_TRANSITIONS[from].includes(to));
      }
    }
  });

  it('terminal phases have no exits; every non-terminal phase can end or fail', () => {
    for (const t of TERMINAL_PHASES) expect(PHASE_TRANSITIONS[t]).toEqual([]);
    for (const p of phases.filter((x) => !TERMINAL_PHASES.has(x))) {
      expect(PHASE_TRANSITIONS[p]).toContain('ended');
      expect(PHASE_TRANSITIONS[p]).toContain('failed');
    }
  });

  it('every activity can reach stopped (stop guarantee) and stopped can recover', () => {
    for (const a of activities) if (a !== 'stopped') expect(ACTIVITY_TRANSITIONS[a]).toContain('stopped');
    expect(ACTIVITY_TRANSITIONS.stopped).toContain('ready');
  });

  it('same-state is a no-op and does not notify', () => {
    const onChange = vi.fn();
    const m = new StateMachine<CallPhase>('idle', PHASE_TRANSITIONS, onChange);
    expect(m.to('idle')).toBe(true);
    expect(onChange).not.toHaveBeenCalled();
  });
});

describe('EventBus + versioned events', () => {
  it('events carry v=1 and a timestamp', () => {
    const e = makeEvent({ type: 'transport.changed', transport: 'p2p' }, 123);
    expect(e).toEqual({ v: ENGINE_EVENT_VERSION, type: 'transport.changed', transport: 'p2p', at: 123 });
  });

  it('delivers by type and to onAny; unsubscribe works', () => {
    const bus = new EventBus();
    const typed = vi.fn();
    const any = vi.fn();
    const off = bus.on('transport.changed', typed);
    bus.onAny(any);
    bus.emit(makeEvent({ type: 'transport.changed', transport: 'relay' }));
    bus.emit(makeEvent({ type: 'permission.changed', state: 'granted' }));
    expect(typed).toHaveBeenCalledTimes(1);
    expect(any).toHaveBeenCalledTimes(2);
    off();
    bus.emit(makeEvent({ type: 'transport.changed', transport: 'p2p' }));
    expect(typed).toHaveBeenCalledTimes(1);
  });

  it('a throwing listener is isolated from the engine and other listeners', () => {
    const errors: unknown[] = [];
    const bus = new EventBus((e) => errors.push(e));
    const after = vi.fn();
    bus.onAny(() => {
      throw new Error('ui bug');
    });
    bus.onAny(after);
    expect(() => bus.emit(makeEvent({ type: 'permission.changed', state: 'denied' }))).not.toThrow();
    expect(after).toHaveBeenCalled();
    expect(errors).toHaveLength(1);
  });

  it('peer messages: version gate and validation', () => {
    expect(isPeerMessage({ v: 1, t: 'hello', lang: 'en', name: 'x' })).toBe(true);
    expect(isPeerMessage({ v: 99, t: 'hello', lang: 'en', name: 'x' })).toBe(false);
    expect(isPeerMessage({ t: 'segment', id: '', original: '', translated: '', sourceLang: 'en', targetLang: 'es', sentAt: 1 })).toBe(false);
  });
});

describe('Diagnostics redaction', () => {
  it('keeps numbers, booleans and enum-like strings under allowed keys only', () => {
    expect(
      redact({
        from: 'idle',
        to: 'connecting',
        ms: 12,
        ok: true,
        text: 'hello there',
        original: 'secret words',
        name: 'Ana',
        apiKey: 'sk_live_123',
        reason: 'has spaces in it',
        code: 'permission-denied',
        nested: { a: 1 },
      })
    ).toEqual({ from: 'idle', to: 'connecting', ms: 12, ok: true, code: 'permission-denied' });
  });

  it('ring buffer is bounded and metrics summarize samples', () => {
    const d = new Diagnostics(5, 3, () => 0);
    for (let i = 0; i < 20; i++) d.record('x', { i });
    d.sample('m', 10);
    d.sample('m', 30);
    d.sample('m', 20);
    d.sample('m', 40); // evicts 10
    const s = d.snapshot();
    expect(s.events).toHaveLength(5);
    expect(s.events[0].i).toBe(15);
    expect(s.metrics.m).toEqual({ n: 3, p50: 30, p95: 40, max: 40 });
  });

  it('rejects unsafe event/metric names', () => {
    const d = new Diagnostics();
    d.record('hello there my secret', {});
    d.count('a b');
    expect(d.snapshot().events).toHaveLength(0);
    expect(d.snapshot().counters).toEqual({});
  });
});

describe('segment identity', () => {
  it('accepts each peer segment id once (dedupe for resends)', () => {
    const l = new SegmentLedger();
    expect(l.acceptPeerSegment('a')).toBe(true);
    expect(l.acceptPeerSegment('a')).toBe(false);
    expect(l.acceptPeerSegment('b')).toBe(true);
  });

  it('sequence numbers are monotonic', () => {
    const l = new SegmentLedger();
    expect([l.nextSeq(), l.nextSeq(), l.nextSeq()]).toEqual([0, 1, 2]);
  });

  it('records and dedupe memory are bounded', () => {
    const l = new SegmentLedger(3, 3);
    for (let i = 0; i < 10; i++) {
      l.upsert({ id: `s${i}`, speaker: 'peer', original: '', translated: '', sourceLang: 'en', targetLang: 'es', translation: 'translated', createdAt: i });
      l.acceptPeerSegment(`s${i}`);
    }
    expect(l.list().map((r) => r.id)).toEqual(['s7', 's8', 's9']);
    const s = new BoundedSet<string>(2);
    s.add('a');
    s.add('b');
    s.add('c');
    expect([s.has('a'), s.has('b'), s.has('c')]).toEqual([false, true, true]);
  });
});
