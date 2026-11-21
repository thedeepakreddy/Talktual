// Minimal browser fakes. They model only the behavior the code under test
// relies on, and record calls so tests can assert on them.

export class FakeWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  static instances: FakeWebSocket[] = [];

  readyState = FakeWebSocket.CONNECTING;
  sent: unknown[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;

  constructor(public url: string) {
    FakeWebSocket.instances.push(this);
  }
  send(data: string) {
    this.sent.push(JSON.parse(data));
  }
  close() {
    if (this.readyState === FakeWebSocket.CLOSED) return;
    this.readyState = FakeWebSocket.CLOSED;
    this.onclose?.();
  }
  // --- test drivers ---
  open() {
    this.readyState = FakeWebSocket.OPEN;
    this.onopen?.();
  }
  receive(msg: unknown) {
    this.onmessage?.({ data: typeof msg === 'string' ? msg : JSON.stringify(msg) });
  }
  drop() {
    this.readyState = FakeWebSocket.CLOSED;
    this.onclose?.();
  }
}

export class FakeDataChannel {
  readyState: RTCDataChannelState = 'connecting';
  sent: unknown[] = [];
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  send(d: string) {
    this.sent.push(JSON.parse(d));
  }
  close() {
    this.readyState = 'closed';
  }
  open() {
    this.readyState = 'open';
    this.onopen?.();
  }
  remoteClose() {
    this.readyState = 'closed';
    this.onclose?.();
  }
}

export class FakePeerConnection {
  static instances: FakePeerConnection[] = [];
  static failSetRemote = false;
  connectionState = 'new';
  channel: FakeDataChannel | null = null;
  ondatachannel: ((e: { channel: FakeDataChannel }) => void) | null = null;
  onicecandidate: ((e: { candidate: unknown }) => void) | null = null;
  onconnectionstatechange: (() => void) | null = null;
  addedCandidates: unknown[] = [];
  constructor() {
    FakePeerConnection.instances.push(this);
  }
  createDataChannel() {
    this.channel = new FakeDataChannel();
    return this.channel;
  }
  async createOffer() {
    return { type: 'offer', sdp: 'o' };
  }
  async createAnswer() {
    return { type: 'answer', sdp: 'a' };
  }
  async setLocalDescription() {}
  async setRemoteDescription() {
    if (FakePeerConnection.failSetRemote) throw new Error('bad sdp');
  }
  async addIceCandidate(c: unknown) {
    this.addedCandidates.push(c);
  }
  close() {
    this.connectionState = 'closed';
  }
}

/** A promise you resolve from the outside. */
export function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

export const flush = async (n = 10) => {
  for (let i = 0; i < n; i++) await Promise.resolve();
};
