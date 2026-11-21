import { describe, it, expect, vi, beforeEach } from 'vitest';
import { FakeWebSocket, FakePeerConnection, flush } from '../test/fakes';
import { connectCall, isCallMessage, CallMessage } from './webrtc';

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
    expect(ws.sent).toContainEqual({ type: 'app', payload: { t: 'hello', lang: 'en-US', name: 'Me' } });
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
    expect(pc.channel!.sent).toEqual([segment]);
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
