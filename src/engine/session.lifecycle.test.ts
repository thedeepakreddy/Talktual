import { describe, it, expect } from 'vitest';
import { makeSession, connected, settle } from '../test/engineFakes';
import type { EngineEvent } from './events';

describe('CommunicationSession lifecycle (M2.1)', () => {
  it('idle → connecting → waiting-for-peer → connected; sends hello; records connectedAt', async () => {
    const h = makeSession({ consent: true });
    const phases: string[] = [];
    h.session.events.on('session.phase', (e) => phases.push(`${e.from}>${e.to}`));
    expect(h.session.getSnapshot().phase).toBe('idle');
    h.session.start();
    h.call.status('waiting-for-peer');
    h.call.status('connected');
    expect(phases).toEqual(['idle>connecting', 'connecting>waiting-for-peer', 'waiting-for-peer>connected']);
    const s = h.session.getSnapshot();
    expect(s.phase).toBe('connected');
    expect(s.connectedAt).not.toBeNull();
    expect(h.call.sent.filter((m) => m.t === 'hello').length).toBeGreaterThanOrEqual(1);
    expect(h.call.sent[0]).toEqual({ t: 'hello', lang: 'en-US', name: 'Ana' });
  });

  it('start() is idempotent', () => {
    const h = makeSession();
    h.session.start();
    h.session.start();
    expect(h.connects).toBe(1);
  });

  it('snapshot is immutable per change and subscribers are notified', () => {
    const h = makeSession();
    let n = 0;
    h.session.subscribe(() => n++);
    const before = h.session.getSnapshot();
    h.session.start();
    expect(h.session.getSnapshot()).not.toBe(before);
    expect(before.phase).toBe('idle');
    expect(n).toBeGreaterThan(0);
  });

  it('peer hello updates the peer and emits peer.updated', async () => {
    const h = makeSession();
    const seen: EngineEvent[] = [];
    h.session.events.on('peer.updated', (e) => seen.push(e));
    await connected(h, 'es-ES');
    expect(h.session.getSnapshot().peer).toEqual({ name: 'Luis', lang: 'es-ES' });
    expect(seen).toHaveLength(1);
  });

  it.each([
    ['peer-left', 'ended', 'peer-left'],
    ['expired', 'ended', 'expired'],
    ['session-full', 'failed', 'session-full'],
    ['invalid-session', 'failed', 'session-invalid'],
    ['resume-failed', 'failed', 'resume-failed'],
    ['disconnected', 'failed', 'connection-lost'],
    ['error', 'failed', 'server-error'],
  ])('transport %s → %s (%s)', async (status, phase, reason) => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.call.status(status);
    const s = h.session.getSnapshot();
    expect(s.phase).toBe(phase);
    expect(s.phaseReason).toBe(reason);
    if (phase === 'failed') expect(s.notice?.sticky).toBe(true);
  });

  it('teardown happens exactly once and releases everything', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.session.pressTalk();
    await settle();
    const asr = h.asr()!;
    h.call.receive({ t: 'segment', id: 'p1', original: 'hola', translated: 'hello', sourceLang: 'es-ES', targetLang: 'en-US', sentAt: 1 });
    h.call.status('peer-left');
    h.session.leave();
    h.session.dispose();
    h.session.dispose();
    expect(h.call.closed).toBe(1);
    expect(asr.aborts).toBe(1);
    expect(asr.disposed).toBe(true);
    expect(h.capture.releases).toBeGreaterThanOrEqual(1);
    expect(h.playback.cancelled).toBe(1);
    expect(h.session.getSnapshot().capturing).toBe(false);
  });

  it('leave(): ends locally, closes transport once, later transport events are ignored', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.session.leave();
    h.session.leave();
    expect(h.session.getSnapshot().phase).toBe('ended');
    expect(h.session.getSnapshot().phaseReason).toBe('local-leave');
    expect(h.call.closed).toBe(1);
    h.call.status('connected');
    h.call.status('error');
    expect(h.session.getSnapshot().phase).toBe('ended');
  });

  it('race: leave while still connecting', () => {
    const h = makeSession();
    h.session.start();
    h.session.leave();
    h.call.status('waiting-for-peer');
    h.call.status('connected');
    expect(h.session.getSnapshot().phase).toBe('ended');
    expect(h.call.closed).toBe(1);
  });

  it('race: a segment arriving after the call ended is never played', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.session.leave();
    h.call.receive({ t: 'segment', id: 'late', original: 'hola', translated: 'hello', sourceLang: 'es-ES', targetLang: 'en-US', sentAt: 1 });
    expect(h.playback.played).toHaveLength(0);
  });

  it('race: a translation finishing after leave is not sent', async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const h = makeSession({
      consent: true,
      translate: async (req) => {
        await gate;
        return { segmentId: req.segmentId, status: 'translated', text: 'tarde', attempts: 1, latencyMs: 1 };
      },
    });
    await connected(h);
    h.session.pressTalk();
    await settle();
    h.asr()!.say('late words');
    h.session.leave();
    release();
    await settle(50);
    expect(h.call.segments()).toHaveLength(0);
  });

  it('commands after end are no-ops', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.session.leave();
    const asrCount = h.asr() ? 1 : 0;
    h.session.pressTalk();
    h.session.setListeningMode('continuous');
    h.session.setLanguage('fr-FR');
    h.session.grantConsent();
    expect(h.asr() ? 1 : 0).toBe(asrCount);
    expect(h.session.getSnapshot().mode).toBe('push-to-talk');
  });

  it('no events or subscriber calls after dispose', async () => {
    const h = makeSession();
    await connected(h);
    let n = 0;
    h.session.subscribe(() => n++);
    h.session.events.onAny(() => n++);
    h.session.dispose();
    const before = n;
    h.call.status('connected');
    h.call.receive({ t: 'hello', lang: 'fr', name: 'x' });
    expect(n).toBe(before);
  });

  it('reconnecting → connected: recovery events, notice cleared, hello re-sent', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    const kinds: string[] = [];
    h.session.events.on('recovery', (e) => kinds.push(e.kind));
    h.call.handlers.onReconnectAttempt?.(1);
    h.call.status('reconnecting');
    expect(h.session.getSnapshot().phase).toBe('reconnecting');
    expect(h.session.getSnapshot().notice?.code).toBe('connection-lost');
    const hellos = h.call.sent.filter((m) => m.t === 'hello').length;
    h.call.status('connected');
    expect(h.session.getSnapshot().phase).toBe('connected');
    expect(h.session.getSnapshot().notice).toBeNull();
    expect(h.session.getSnapshot().reconnectAttempt).toBe(0);
    expect(h.call.sent.filter((m) => m.t === 'hello').length).toBe(hellos + 1);
    expect(kinds).toEqual(['reconnecting', 'reconnected']);
  });

  it('peer-away: waiting with a sticky notice, capture stopped; peer-back + ready resumes', async () => {
    const h = makeSession({ consent: true });
    await connected(h);
    h.session.pressTalk();
    await settle();
    const asr = h.asr()!;
    h.call.status('peer-away');
    expect(h.session.getSnapshot().phase).toBe('waiting-for-peer');
    expect(h.session.getSnapshot().notice?.code).toBe('peer-away');
    expect(asr.active).toBe(false);
    expect(h.session.getSnapshot().holding).toBe(false);
    h.call.status('peer-back');
    h.call.status('connected');
    expect(h.session.getSnapshot().phase).toBe('connected');
    expect(h.session.getSnapshot().notice).toBeNull();
  });

  it('illegal transitions are refused and counted, not thrown', async () => {
    const h = makeSession();
    h.session.start();
    h.call.status('peer-back'); // no phase change
    h.call.status('connected');
    h.call.status('peer-left');
    h.call.status('reconnecting'); // ended → reconnecting is illegal; ignored
    expect(h.session.getSnapshot().phase).toBe('ended');
  });
});
