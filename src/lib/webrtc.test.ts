import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { FakeWebSocket, FakePeerConnection, flush } from '../test/fakes';
import { connectCall, isCallMessage, CallMessage, reconnectDelay, OUTBOX_MAX_SEGMENTS } from './webrtc';

beforeEach(() => {
  FakeWebSocket.instances = [];
  FakePeerConnection.instances = [];
  FakePeerConnection.failSetRemote = false;
  vi.stubGlobal('WebSocket', FakeWebSocket);
  vi.stubGlobal('RTCPeerConnection', FakePeerConnection);
});

const segment: CallMessage = {
  t: 'segment',
  id: 'x1',
  original: 'hello',
  translated: 'hola',
  sourceLang: 'en-US',
  targetLang: 'es-ES',
  sentAt: 1,
};

function setup() {
  const onMessage = vi.fn();
  const onStatus = vi.fn();
  const onTransport = vi.fn();
  const call = connectCall('ROOM01', 'ws://test', { onMessage, onStatus, onTransport });
  const ws = FakeWebSocket.instances[0];
  const pc = FakePeerConnection.instances[0];
  return { call, ws, pc, onMessage, onStatus, onTransport };
}

describe('isCallMessage (DEF-C06)', () => {
  it('accepts the three protocol messages', () => {
    expect(isCallMessage({ t: 'hello', lang: 'es-ES', name: 'Ana' })).toBe(true);
    expect(isCallMessage({ t: 'partial', text: '', lang: 'en' })).toBe(true);
    expect(isCallMessage(segment)).toBe(true);
    expect(isCallMessage({ ...segment, untranslated: true })).toBe(true);
  });

  it.each([
    null,
    undefined,
    'str',
    {},
    { t: 'segment' },
    { ...segment, targetLang: undefined },
    { ...segment, original: 5 },
    { ...segment, untranslated: 'yes' },
    { ...segment, translated: 'x'.repeat(5000) },
    { t: 'hello', lang: 'en' },
    { t: 'hello', lang: 'en', name: 'n'.repeat(500) },
    { t: 'unknown' },
  ])('rejects %j', (v) => {
    expect(isCallMessage(v)).toBe(false);
  });
});

describe('connectCall', () => {
  it('joins on open, goes live on ready over the relay, and flushes the outbox', () => {
    const { call, ws, onStatus, onTransport } = setup();
    call.send({ t: 'hello', lang: 'en-US', name: 'Me' });
    ws.open();
    expect(ws.sent[0]).toEqual({ type: 'join', sessionId: 'ROOM01' });
    expect(onStatus).toHaveBeenLastCalledWith('waiting-for-peer');
    ws.receive({ type: 'role', role: 'answerer' });
    ws.receive({ type: 'ready' });
    expect(onTransport).toHaveBeenCalledWith('relay');
    expect(onStatus).toHaveBeenLastCalledWith('connected');
    expect(ws.sent).toContainEqual({ type: 'app', payload: { v: 1, t: 'hello', lang: 'en-US', name: 'Me' } });
  });

  it('delivers valid relayed messages and drops malformed ones without throwing', async () => {
    const { ws, onMessage } = setup();
    ws.open();
    const rejections: unknown[] = [];
    const onRej = (e: PromiseRejectionEvent) => rejections.push(e.reason);
    window.addEventListener('unhandledrejection', onRej);
    ws.receive({ type: 'app' }); // payload missing: used to throw in the handler
    ws.receive({ type: 'app', payload: { t: 'segment', id: 'x' } });
    ws.receive('not json');
    ws.receive({ type: 'app', payload: segment });
    await flush();
    window.removeEventListener('unhandledrejection', onRej);
    expect(onMessage).toHaveBeenCalledTimes(1);
    expect(onMessage).toHaveBeenCalledWith(segment);
    expect(rejections).toEqual([]);
  });

  it('offerer opens a data channel, upgrades to p2p, and uses it for sends', async () => {
    const { call, ws, pc, onTransport, onMessage } = setup();
    ws.open();
    ws.receive({ type: 'role', role: 'offerer' });
    ws.receive({ type: 'ready' });
    await flush();
    expect(ws.sent).toContainEqual({ type: 'offer', offer: { type: 'offer', sdp: 'o' } });
    pc.channel!.open();
    expect(onTransport).toHaveBeenLastCalledWith('p2p');
    call.send(segment);
    expect(pc.channel!.sent).toEqual([{ ...segment, v: 1 }]);
    pc.channel!.onmessage!({ data: JSON.stringify(segment) });
    pc.channel!.onmessage!({ data: JSON.stringify({ t: 'segment' }) });
    expect(onMessage).toHaveBeenCalledTimes(1);
  });

  it('queues ICE until the remote description is set', async () => {
    const { ws, pc } = setup();
    ws.open();
    ws.receive({ type: 'ice', candidate: { candidate: 'c1' } });
    expect(pc.addedCandidates).toEqual([]);
    ws.receive({ type: 'offer', offer: { type: 'offer', sdp: 'o' } });
    await flush();
    expect(pc.addedCandidates).toEqual([{ candidate: 'c1' }]);
    expect(ws.sent).toContainEqual({ type: 'answer', answer: { type: 'answer', sdp: 'a' } });
  });

  it('a failed negotiation stays on the relay without an unhandled rejection', async () => {
    FakePeerConnection.failSetRemote = true;
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const { ws, onStatus } = setup();
    ws.open();
    ws.receive({ type: 'ready' });
    ws.receive({ type: 'offer', offer: { type: 'offer', sdp: 'bad' } });
    await flush();
    expect(warn).toHaveBeenCalled();
    expect(onStatus).toHaveBeenLastCalledWith('connected');
  });

  it('DEF-C07: data channel closing after the signaling socket dropped reports disconnected', async () => {
    const { ws, pc, onStatus } = setup();
    ws.open();
    ws.receive({ type: 'role', role: 'offerer' });
    ws.receive({ type: 'ready' });
    await flush();
    pc.channel!.open();
    ws.drop(); // P2P still up, so no status change yet
    expect(onStatus).toHaveBeenLastCalledWith('connected');
    pc.channel!.remoteClose(); // now nothing can carry messages
    expect(onStatus).toHaveBeenLastCalledWith('disconnected');
  });

  it('maps server statuses and stays quiet after close()', () => {
    const { call, ws, onStatus } = setup();
    ws.open();
    ws.receive({ type: 'peer-left' });
    expect(onStatus).toHaveBeenLastCalledWith('peer-left');
    ws.receive({ type: 'expired' });
    expect(onStatus).toHaveBeenLastCalledWith('expired');
    call.close();
    onStatus.mockClear();
    ws.drop();
    expect(onStatus).not.toHaveBeenCalled();
    call.send(segment); // no throw, nothing sent
  });
});

describe('Phase 2 transport: versioning, leave, outbox, resume', () => {
  it('rejects unknown wire versions but accepts unversioned (Phase 1) and v1', () => {
    expect(isCallMessage({ ...segment, v: 1 })).toBe(true);
    expect(isCallMessage(segment)).toBe(true);
    expect(isCallMessage({ ...segment, v: 2 })).toBe(false);
    expect(isCallMessage({ ...segment, seq: -1 })).toBe(false);
    expect(isCallMessage({ ...segment, seq: 3 })).toBe(true);
  });

  it('close() sends an intentional leave before closing', () => {
    const { call, ws } = setup();
    ws.open();
    call.close();
    expect(ws.sent.at(-1)).toEqual({ type: 'leave' });
    expect(ws.readyState).toBe(FakeWebSocket.CLOSED);
  });

  it('does not relay into an empty room: holds segments, drops partials, keeps the latest hello', () => {
    const { call, ws } = setup();
    ws.open(); // joined, but no peer yet
    call.send({ t: 'hello', lang: 'en', name: 'A' });
    call.send({ t: 'hello', lang: 'fr', name: 'A' });
    call.send({ t: 'partial', text: 'stale', lang: 'en' });
    call.send(segment);
    expect(ws.sent.filter((m) => (m as { type: string }).type === 'app')).toEqual([]);
    ws.receive({ type: 'ready' });
    const apps = ws.sent.filter((m) => (m as { type: string }).type === 'app').map((m) => (m as { payload: CallMessage }).payload);
    expect(apps).toEqual([
      { v: 1, t: 'hello', lang: 'fr', name: 'A' },
      { ...segment, v: 1 },
    ]);
  });

  it('outbox is bounded: keeps the newest segments', () => {
    const { call, ws } = setup();
    for (let i = 0; i < OUTBOX_MAX_SEGMENTS + 5; i++) call.send({ ...segment, id: `s${i}` });
    ws.open();
    ws.receive({ type: 'ready' });
    const ids = ws.sent
      .filter((m) => (m as { type: string }).type === 'app')
      .map((m) => ((m as { payload: { id: string } }).payload.id));
    expect(ids).toHaveLength(OUTBOX_MAX_SEGMENTS);
    expect(ids[0]).toBe('s5');
  });

  it('maps server error codes and peer presence', () => {
    const { ws, onStatus } = setup();
    ws.open();
    ws.receive({ type: 'error', code: 'session_full' });
    expect(onStatus).toHaveBeenLastCalledWith('session-full');
    ws.receive({ type: 'error', code: 'invalid_session' });
    expect(onStatus).toHaveBeenLastCalledWith('invalid-session');
    ws.receive({ type: 'peer-away' });
    expect(onStatus).toHaveBeenLastCalledWith('peer-away');
    ws.receive({ type: 'peer-back' });
    expect(onStatus).toHaveBeenLastCalledWith('peer-back');
  });

  it('backoff doubles and is capped', () => {
    expect([1, 2, 3, 4, 5, 6, 7].map((n) => reconnectDelay(n))).toEqual([500, 1000, 2000, 4000, 8000, 8000, 8000]);
  });

  describe('reconnect', () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    function joinedCall() {
      const ctx = setup();
      ctx.ws.open();
      ctx.ws.receive({ type: 'role', role: 'answerer' });
      ctx.ws.receive({ type: 'joined', role: 'answerer', resumeToken: 'tok-1' });
      ctx.ws.receive({ type: 'ready' });
      return ctx;
    }

    it('signaling drop without P2P → reconnecting, rejoins with the token, renegotiates, resends held segments', async () => {
      const ctx = joinedCall();
      ctx.ws.drop();
      expect(ctx.onStatus).toHaveBeenLastCalledWith('reconnecting');
      ctx.call.send(segment); // produced while offline → held
      vi.advanceTimersByTime(500);
      const ws2 = FakeWebSocket.instances[1];
      expect(ws2).toBeDefined();
      ws2.open();
      expect(ws2.sent[0]).toEqual({ type: 'join', sessionId: 'ROOM01', resumeToken: 'tok-1' });
      ws2.receive({ type: 'joined', role: 'answerer', resumeToken: 'tok-1', resumed: true });
      ws2.receive({ type: 'ready', resumed: true });
      expect(ctx.onStatus).toHaveBeenLastCalledWith('connected');
      expect(ws2.sent).toContainEqual({ type: 'app', payload: { ...segment, v: 1 } });
      // A fresh peer connection is used for renegotiation.
      ws2.receive({ type: 'offer', offer: { type: 'offer', sdp: 'o2' } });
      await flush();
      expect(FakePeerConnection.instances.length).toBe(2);
      expect(ws2.sent).toContainEqual({ type: 'answer', answer: { type: 'answer', sdp: 'a' } });
    });

    it('signaling drop with P2P up → call stays connected while signaling recovers in the background', async () => {
      const ctx = setup();
      ctx.ws.open();
      ctx.ws.receive({ type: 'role', role: 'offerer' });
      ctx.ws.receive({ type: 'joined', role: 'offerer', resumeToken: 'tok-2' });
      ctx.ws.receive({ type: 'ready' });
      await flush();
      ctx.pc.channel!.open();
      ctx.onStatus.mockClear();
      ctx.ws.drop();
      expect(ctx.onStatus).not.toHaveBeenCalled();
      ctx.call.send(segment);
      expect(ctx.pc.channel!.sent).toContainEqual({ ...segment, v: 1 });
      vi.advanceTimersByTime(500);
      expect(FakeWebSocket.instances).toHaveLength(2);
    });

    it('gives up after the attempt budget and reports disconnected', () => {
      const ctx = joinedCall();
      const attempts = vi.fn();
      ctx.ws.drop();
      for (let i = 0; i < 6; i++) {
        vi.advanceTimersByTime(10_000);
        FakeWebSocket.instances.at(-1)!.drop(); // each attempt fails
      }
      vi.advanceTimersByTime(10_000);
      expect(FakeWebSocket.instances).toHaveLength(7);
      expect(ctx.onStatus).toHaveBeenLastCalledWith('disconnected');
      void attempts;
    });

    it('resume refused → resume-failed, no further attempts', () => {
      const ctx = joinedCall();
      ctx.ws.drop();
      vi.advanceTimersByTime(500);
      const ws2 = FakeWebSocket.instances[1];
      ws2.open();
      ws2.receive({ type: 'error', code: 'resume_failed' });
      expect(ctx.onStatus).toHaveBeenLastCalledWith('resume-failed');
      ws2.drop();
      vi.advanceTimersByTime(60_000);
      expect(FakeWebSocket.instances).toHaveLength(2);
    });

    it('leaving during reconnect cancels the pending attempt', () => {
      const ctx = joinedCall();
      ctx.ws.drop();
      ctx.call.close();
      vi.advanceTimersByTime(60_000);
      expect(FakeWebSocket.instances).toHaveLength(1);
    });

    it('without a resume token (Phase 1 server) a drop is final, as before', () => {
      const ctx = setup();
      ctx.ws.open();
      ctx.ws.receive({ type: 'ready' });
      ctx.ws.drop();
      expect(ctx.onStatus).toHaveBeenLastCalledWith('disconnected');
      vi.advanceTimersByTime(60_000);
      expect(FakeWebSocket.instances).toHaveLength(1);
    });
  });
});
