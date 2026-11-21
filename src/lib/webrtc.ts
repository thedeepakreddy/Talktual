// Call transport.
//
// Call text travels over an RTCDataChannel rather than being relayed through
// the signaling server: once ICE settles the two browsers talk directly, which
// removes a server round trip from a path where every hop is audible. The
// WebSocket stays connected and acts as a fallback transport until (or if) the
// data channel opens.
//
// No media tracks are negotiated. The peer never hears the raw microphone —
// only the synthesized translation — so sending the original audio would just
// burn bandwidth and feed echo back into the mic.
//
// Phase 2 additions: versioned wire messages, a resume token so a dropped
// signaling socket can reclaim its seat (with exponential backoff and P2P
// renegotiation), an explicit `leave`, peer-away/back, and a bounded outbox.

import { isPeerMessage, PeerMessage, WIRE_VERSION } from '../engine/events';

/** Phase 1 names, kept for existing callers. */
export type CallMessage = PeerMessage;
export const isCallMessage = isPeerMessage;

export type Transport = 'p2p' | 'relay';

export type TransportStatus =
  | 'waiting-for-peer'
  | 'connected'
  | 'peer-left'
  | 'peer-away'
  | 'peer-back'
  | 'expired'
  | 'error'
  | 'session-full'
  | 'invalid-session'
  | 'resume-failed'
  | 'reconnecting'
  | 'disconnected';

export interface CallHandlers {
  onMessage: (msg: CallMessage) => void;
  onStatus: (status: TransportStatus | string) => void;
  onTransport: (transport: Transport) => void;
  /** Reconnect attempt n (1-based) is about to start. */
  onReconnectAttempt?: (attempt: number) => void;
}

export interface CallHandle {
  send: (msg: CallMessage) => void;
  /** Intentional leave: tells the server, so the peer is notified at once. */
  close: () => void;
}

export interface ReconnectPolicy {
  maxAttempts: number;
  baseDelayMs: number;
  maxDelayMs: number;
}

export const DEFAULT_RECONNECT: ReconnectPolicy = { maxAttempts: 6, baseDelayMs: 500, maxDelayMs: 8000 };
/** Segments held while no transport can carry them. Partials are never queued. */
export const OUTBOX_MAX_SEGMENTS = 20;

const ICE_SERVERS: RTCIceServer[] = [
  { urls: ['stun:stun.l.google.com:19302', 'stun:stun1.l.google.com:19302'] },
];

// STUN alone fails behind symmetric NAT (common on mobile carriers and
// corporate networks). A TURN server is the fix; without one those calls fall
// back to the relay transport, which is fine here because we only ship text.
const TURN_URL = import.meta.env.VITE_TURN_URL;
if (TURN_URL) {
  ICE_SERVERS.push({
    urls: TURN_URL,
    username: import.meta.env.VITE_TURN_USERNAME,
    credential: import.meta.env.VITE_TURN_CREDENTIAL,
  });
}

export function reconnectDelay(attempt: number, policy: ReconnectPolicy = DEFAULT_RECONNECT): number {
  return Math.min(policy.maxDelayMs, policy.baseDelayMs * 2 ** Math.max(0, attempt - 1));
}

export function connectCall(
  sessionId: string,
  signalingUrl: string,
  handlers: CallHandlers,
  options: { reconnect?: ReconnectPolicy | false } = {}
): CallHandle {
  const policy = options.reconnect === false ? null : (options.reconnect ?? DEFAULT_RECONNECT);

  let ws: WebSocket;
  let pc: RTCPeerConnection;
  let role: 'offerer' | 'answerer' | null = null;
  let channel: RTCDataChannel | null = null;
  let remoteDescriptionSet = false;
  let pendingCandidates: RTCIceCandidateInit[] = [];
  let closed = false;
  let resumeToken: string | null = null;
  let peerPresent = false;
  let attempt = 0;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let reconnecting = false;
  let gaveUp = false;

  // Messages produced while nothing can carry them.
  let pendingHello: CallMessage | null = null;
  let outbox: CallMessage[] = [];

  const channelOpen = () => channel?.readyState === 'open';
  const wsOpen = () => ws?.readyState === WebSocket.OPEN;

  function wsSend(payload: unknown) {
    if (wsOpen()) ws.send(JSON.stringify(payload));
  }

  function flushOutbox() {
    const queued = [...(pendingHello ? [pendingHello] : []), ...outbox];
    pendingHello = null;
    outbox = [];
    queued.forEach(send);
  }

  function send(msg: CallMessage) {
    if (closed) return;
    const stamped = { ...msg, v: WIRE_VERSION } as CallMessage;
    if (channelOpen()) {
      channel!.send(JSON.stringify(stamped));
      return;
    }
    if (wsOpen() && peerPresent) {
      wsSend({ type: 'app', payload: stamped });
      return;
    }
    // Nothing can deliver it right now.
    if (stamped.t === 'hello') pendingHello = stamped;
    else if (stamped.t === 'segment') {
      outbox.push(stamped);
      if (outbox.length > OUTBOX_MAX_SEGMENTS) outbox.shift();
    }
    // Partials are live captions: stale by the time anything could send them.
  }

  function deliver(value: unknown) {
    if (isCallMessage(value)) handlers.onMessage(value);
  }

  function attachChannel(dc: RTCDataChannel) {
    channel = dc;
    dc.onopen = () => {
      if (channel !== dc) return;
      handlers.onTransport('p2p');
      handlers.onStatus('connected');
      flushOutbox();
    };
    dc.onclose = () => {
      if (closed || channel !== dc) return;
      handlers.onTransport('relay');
      // The relay is only a fallback if the signaling socket is still up.
      if (!wsOpen()) {
        if (reconnecting) handlers.onStatus('reconnecting');
        else handlers.onStatus('disconnected');
      }
    };
    dc.onmessage = (event) => {
      let parsed: unknown;
      try {
        parsed = JSON.parse(event.data);
      } catch {
        return; /* ignore malformed frames */
      }
      deliver(parsed);
    };
  }

  function createPeer() {
    const p = new RTCPeerConnection({ iceServers: ICE_SERVERS });
    p.ondatachannel = (event) => attachChannel(event.channel);
    p.onicecandidate = (event) => {
      if (event.candidate) wsSend({ type: 'ice', candidate: event.candidate });
    };
    // Peer-connection state only decides *which* transport we use, never whether
    // the call is up — that is the signaling socket's job.
    p.onconnectionstatechange = () => {
      if (p !== pc) return;
      const state = p.connectionState;
      if (state === 'connected' && channelOpen()) handlers.onTransport('p2p');
      else if ((state === 'failed' || state === 'disconnected' || state === 'closed') && !closed) {
        handlers.onTransport('relay');
      }
    };
    return p;
  }

  function resetPeer() {
    try {
      channel?.close();
    } catch {
      /* ignore */
    }
    channel = null;
    try {
      pc?.close();
    } catch {
      /* ignore */
    }
    remoteDescriptionSet = false;
    pendingCandidates = [];
    pc = createPeer();
  }

  function drainCandidates() {
    pendingCandidates.forEach((c) => pc.addIceCandidate(c).catch(() => {}));
    pendingCandidates = [];
  }

  async function handleSignal(msg: Record<string, unknown>) {
    switch (msg.type) {
      case 'role':
        role = msg.role === 'answerer' ? 'answerer' : 'offerer';
        break;

      case 'joined':
        if (typeof msg.resumeToken === 'string') resumeToken = msg.resumeToken;
        if (msg.role === 'answerer' || msg.role === 'offerer') role = msg.role;
        if (reconnecting) {
          reconnecting = false;
          attempt = 0;
          // Seat reclaimed; the peer may still be away (status follows).
          if (!peerPresent) handlers.onStatus('waiting-for-peer');
        }
        break;

      case 'ready': {
        peerPresent = true;
        const resumed = msg.resumed === true;
        // Both peers are in the room, so the relay can already carry text.
        // That makes the call usable now; ICE only upgrades the transport.
        if (!channelOpen()) handlers.onTransport('relay');
        handlers.onStatus('connected');
        flushOutbox();
        if (resumed && channelOpen()) break; // P2P survived the blip
        if (resumed) resetPeer();
        if (role === 'offerer') {
          // Must exist before the offer so it lands in the SDP.
          attachChannel(pc.createDataChannel('talktual', { ordered: true }));
          const offer = await pc.createOffer();
          await pc.setLocalDescription(offer);
          wsSend({ type: 'offer', offer });
        }
        break;
      }

      case 'offer': {
        if (!msg.offer || typeof msg.offer !== 'object') break;
        if (remoteDescriptionSet) resetPeer(); // renegotiation after a resume
        await pc.setRemoteDescription(msg.offer as RTCSessionDescriptionInit);
        remoteDescriptionSet = true;
        drainCandidates();
        const answer = await pc.createAnswer();
        await pc.setLocalDescription(answer);
        wsSend({ type: 'answer', answer });
        break;
      }

      case 'answer':
        if (!msg.answer || typeof msg.answer !== 'object') break;
        await pc.setRemoteDescription(msg.answer as RTCSessionDescriptionInit);
        remoteDescriptionSet = true;
        drainCandidates();
        break;

      case 'ice': {
        const candidate = msg.candidate as RTCIceCandidateInit;
        if (!candidate || typeof candidate !== 'object') break;
        if (remoteDescriptionSet) pc.addIceCandidate(candidate).catch(() => {});
        else pendingCandidates.push(candidate);
        break;
      }

      case 'app':
        // Relay fallback for anything sent before the data channel opened.
        deliver(msg.payload);
        break;

      case 'peer-left':
        peerPresent = false;
        handlers.onStatus('peer-left');
        break;

      case 'peer-away':
        peerPresent = false;
        handlers.onStatus('peer-away');
        break;

      case 'peer-back':
        handlers.onStatus('peer-back');
        break;

      case 'expired':
        handlers.onStatus('expired');
        break;

      case 'error':
        if (msg.code === 'session_full') handlers.onStatus('session-full');
        else if (msg.code === 'invalid_session') handlers.onStatus('invalid-session');
        else if (msg.code === 'resume_failed') {
          gaveUp = true;
          reconnecting = false;
          handlers.onStatus('resume-failed');
        } else handlers.onStatus('error');
        break;
    }
  }

  function scheduleReconnect() {
    if (closed || gaveUp || !policy || !resumeToken) return false;
    if (attempt >= policy.maxAttempts) {
      gaveUp = true;
      reconnecting = false;
      if (!channelOpen()) handlers.onStatus('disconnected');
      return true;
    }
    attempt++;
    reconnecting = true;
    const delay = reconnectDelay(attempt, policy);
    handlers.onReconnectAttempt?.(attempt);
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      if (!closed) openSocket();
    }, delay);
    return true;
  }

  function openSocket() {
    const socket = new WebSocket(signalingUrl);
    ws = socket;
    let opened = false;

    socket.onopen = () => {
      if (ws !== socket) return;
      opened = true;
      socket.send(JSON.stringify(resumeToken ? { type: 'join', sessionId, resumeToken } : { type: 'join', sessionId }));
      if (!reconnecting) handlers.onStatus('waiting-for-peer');
    };

    socket.onmessage = (event) => {
      if (ws !== socket) return;
      let msg: Record<string, unknown>;
      try {
        msg = JSON.parse(event.data);
      } catch {
        return;
      }
      if (!msg || typeof msg !== 'object') return;
      // Negotiation failures only cost us the P2P upgrade — the relay keeps the
      // call working — so they are logged rather than left as unhandled rejections.
      handleSignal(msg).catch((err) => console.warn('WebRTC negotiation failed; staying on relay:', err));
    };

    socket.onclose = () => {
      if (closed || ws !== socket) return;
      peerPresent = false;
      const hadSeat = Boolean(resumeToken);
      if (hadSeat && !gaveUp && policy) {
        // Keep the call on P2P if it's up; otherwise tell the UI we're recovering.
        if (!channelOpen() && !reconnecting) handlers.onStatus('reconnecting');
        scheduleReconnect();
        return;
      }
      if (!channelOpen()) handlers.onStatus('disconnected');
    };

    socket.onerror = () => {
      if (closed || ws !== socket) return;
      // A failed (re)connect attempt is followed by onclose, which handles it.
      if (!opened && !reconnecting && !resumeToken) handlers.onStatus('error');
    };
  }

  pc = createPeer();
  openSocket();

  return {
    send,
    close() {
      if (closed) return;
      closed = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      // Tell the server this is intentional so the peer hears about it now,
      // not after the seat-hold grace period.
      try {
        wsSend({ type: 'leave' });
      } catch {
        /* ignore */
      }
      try {
        channel?.close();
      } catch {
        /* ignore */
      }
      try {
        ws.close();
      } catch {
        /* ignore */
      }
      pc.close();
    },
  };
}
