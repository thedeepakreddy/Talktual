// Peer connection setup.
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

export type CallMessage =
  | { t: "hello"; lang: string; name: string }
  | { t: "partial"; text: string; lang: string }
  | {
      t: "segment";
      id: string;
      original: string;
      translated: string;
      sourceLang: string;
      targetLang: string;
      sentAt: number;
      /** Set when the sender's translation failed and `translated` is just the original. */
      untranslated?: boolean;
    };

export type Transport = "p2p" | "relay";

const MAX_TEXT = 4000;
const isStr = (v: unknown, max: number): v is string => typeof v === "string" && v.length <= max;

/**
 * Runtime check for messages from the other peer. Everything arriving over
 * the data channel or the relay is untrusted input: a malformed frame used to
 * throw inside the async socket handler (unhandled rejection) and an
 * unexpected shape could break rendering.
 */
export function isCallMessage(value: unknown): value is CallMessage {
  if (!value || typeof value !== "object") return false;
  const m = value as Record<string, unknown>;
  switch (m.t) {
    case "hello":
      return isStr(m.lang, 35) && isStr(m.name, 80);
    case "partial":
      return isStr(m.text, MAX_TEXT) && isStr(m.lang, 35);
    case "segment":
      return (
        isStr(m.id, 64) &&
        isStr(m.original, MAX_TEXT) &&
        isStr(m.translated, MAX_TEXT) &&
        isStr(m.sourceLang, 35) &&
        isStr(m.targetLang, 35) &&
        typeof m.sentAt === "number" &&
        (m.untranslated === undefined || typeof m.untranslated === "boolean")
      );
    default:
      return false;
  }
}

export interface CallHandlers {
  onMessage: (msg: CallMessage) => void;
  onStatus: (status: string) => void;
  onTransport: (transport: Transport) => void;
}

export interface CallHandle {
  send: (msg: CallMessage) => void;
  close: () => void;
}

const ICE_SERVERS: RTCIceServer[] = [
  { urls: ["stun:stun.l.google.com:19302", "stun:stun1.l.google.com:19302"] },
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

export function connectCall(
  sessionId: string,
  signalingUrl: string,
  handlers: CallHandlers
): CallHandle {
  const ws = new WebSocket(signalingUrl);
  const pc = new RTCPeerConnection({ iceServers: ICE_SERVERS });

  let role: "offerer" | "answerer" | null = null;
  let channel: RTCDataChannel | null = null;
  let remoteDescriptionSet = false;
  let pendingCandidates: RTCIceCandidateInit[] = [];
  let closed = false;
  // Messages produced before any transport is ready (the hello, usually).
  let outbox: CallMessage[] = [];

  function wsSend(payload: unknown) {
    if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(payload));
  }

  function flushOutbox() {
    if (outbox.length === 0) return;
    const queued = outbox;
    outbox = [];
    queued.forEach(send);
  }

  function deliver(value: unknown) {
    if (isCallMessage(value)) handlers.onMessage(value);
  }

  function send(msg: CallMessage) {
    if (closed) return;
    if (channel?.readyState === "open") {
      channel.send(JSON.stringify(msg));
      return;
    }
    if (ws.readyState === WebSocket.OPEN) {
      wsSend({ type: "app", payload: msg });
      return;
    }
    outbox.push(msg);
  }

  function attachChannel(dc: RTCDataChannel) {
    channel = dc;
    dc.onopen = () => {
      handlers.onTransport("p2p");
      handlers.onStatus("connected");
      flushOutbox();
    };
    dc.onclose = () => {
      if (closed) return;
      handlers.onTransport("relay");
      // The relay is only a fallback if the signaling socket is still up.
      // Otherwise nothing can carry messages any more: say so instead of
      // silently queueing every later turn into the outbox.
      if (ws.readyState !== WebSocket.OPEN) handlers.onStatus("disconnected");
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

  pc.ondatachannel = (event) => attachChannel(event.channel);

  pc.onicecandidate = (event) => {
    if (event.candidate) wsSend({ type: "ice", candidate: event.candidate });
  };

  // Peer-connection state only decides *which* transport we use, never whether
  // the call is up — that is the signaling socket's job. A failed ICE
  // negotiation silently demotes us to the relay instead of killing the call.
  pc.onconnectionstatechange = () => {
    const state = pc.connectionState;
    if (state === "connected" && channel?.readyState === "open") {
      handlers.onTransport("p2p");
    } else if (state === "failed" || state === "disconnected" || state === "closed") {
      if (!closed) handlers.onTransport("relay");
    }
  };

  ws.onopen = () => {
    wsSend({ type: "join", sessionId });
    handlers.onStatus("waiting-for-peer");
  };

  ws.onmessage = (event) => {
    let msg: Record<string, unknown>;
    try {
      msg = JSON.parse(event.data);
    } catch {
      return;
    }
    if (!msg || typeof msg !== "object") return;
    // Negotiation failures only cost us the P2P upgrade — the relay keeps the
    // call working — so they are logged rather than left as unhandled rejections.
    handleSignal(msg).catch((err) => console.warn("WebRTC negotiation failed; staying on relay:", err));
  };

  async function handleSignal(msg: Record<string, unknown>) {

    switch (msg.type) {
      case "role":
        role = msg.role as "offerer" | "answerer";
        break;

      case "ready": {
        // Both peers are in the room, so the relay can already carry text.
        // That makes the call usable now; ICE only upgrades the transport.
        handlers.onTransport("relay");
        handlers.onStatus("connected");
        flushOutbox();
        if (role === "offerer") {
          // Must exist before the offer so it lands in the SDP.
          attachChannel(pc.createDataChannel("talktual", { ordered: true }));
          const offer = await pc.createOffer();
          await pc.setLocalDescription(offer);
          wsSend({ type: "offer", offer });
        }
        break;
      }

      case "offer": {
        await pc.setRemoteDescription(msg.offer as RTCSessionDescriptionInit);
        remoteDescriptionSet = true;
        drainCandidates();
        const answer = await pc.createAnswer();
        await pc.setLocalDescription(answer);
        wsSend({ type: "answer", answer });
        break;
      }

      case "answer":
        await pc.setRemoteDescription(msg.answer as RTCSessionDescriptionInit);
        remoteDescriptionSet = true;
        drainCandidates();
        break;

      case "ice": {
        const candidate = msg.candidate as RTCIceCandidateInit;
        if (!candidate || typeof candidate !== "object") break;
        if (remoteDescriptionSet) pc.addIceCandidate(candidate).catch(() => {});
        else pendingCandidates.push(candidate);
        break;
      }

      case "app":
        // Relay fallback for anything sent before the data channel opened.
        deliver(msg.payload);
        break;

      case "peer-left":
        handlers.onStatus("peer-left");
        break;

      case "expired":
        handlers.onStatus("expired");
        break;

      case "error":
        handlers.onStatus("error");
        break;
    }
  }

  function drainCandidates() {
    pendingCandidates.forEach((c) => pc.addIceCandidate(c).catch(() => {}));
    pendingCandidates = [];
  }

  ws.onclose = () => {
    if (!closed && channel?.readyState !== "open") handlers.onStatus("disconnected");
  };
  ws.onerror = () => {
    if (!closed) handlers.onStatus("error");
  };

  return {
    send,
    close() {
      closed = true;
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
