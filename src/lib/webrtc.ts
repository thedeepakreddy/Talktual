// WebRTC peer connection setup. Handles the offerer/answerer roles assigned
// by the signaling server, so only one side creates the initial offer.

export type SignalMessage =
  | { type: "role"; role: "offerer" | "answerer" }
  | { type: "ready" }
  | { type: "offer"; offer: RTCSessionDescriptionInit }
  | { type: "answer"; answer: RTCSessionDescriptionInit }
  | { type: "ice"; candidate: RTCIceCandidateInit }
  | { type: "peer-left" }
  | { type: "expired" }
  | { type: "error"; message: string }
  | { type: "translated-text"; text: string; lang: string };

export function connectCall(
  sessionId: string,
  signalingUrl: string,
  onRemoteStream: (stream: MediaStream) => void,
  onTranslatedText: (text: string, lang: string) => void,
  onStatusChange: (status: string) => void
) {
  const ws = new WebSocket(signalingUrl);
  const pc = new RTCPeerConnection({
    iceServers: [{ urls: "stun:stun.l.google.com:19302" }],
  });

  let role: "offerer" | "answerer" | null = null;

  pc.ontrack = (event) => onRemoteStream(event.streams[0]);

  pc.onconnectionstatechange = () => {
    onStatusChange(pc.connectionState);
  };

  pc.onicecandidate = (event) => {
    if (event.candidate) {
      ws.send(JSON.stringify({ type: "ice", candidate: event.candidate }));
    }
  };

  ws.onopen = () => {
    ws.send(JSON.stringify({ type: "join", sessionId }));
    onStatusChange("waiting-for-peer");
  };

  ws.onmessage = async (event) => {
    const msg: SignalMessage = JSON.parse(event.data);

    switch (msg.type) {
      case "role":
        role = msg.role;
        break;

      case "ready":
        onStatusChange("connecting");
        if (role === "offerer") {
          const offer = await pc.createOffer();
          await pc.setLocalDescription(offer);
          ws.send(JSON.stringify({ type: "offer", offer }));
        }
        break;

      case "offer":
        await pc.setRemoteDescription(msg.offer);
        const answer = await pc.createAnswer();
        await pc.setLocalDescription(answer);
        ws.send(JSON.stringify({ type: "answer", answer }));
        break;

      case "answer":
        await pc.setRemoteDescription(msg.answer);
        break;

      case "ice":
        try {
          await pc.addIceCandidate(msg.candidate);
        } catch {
          // benign if it arrives before remote description is set
        }
        break;

      case "translated-text":
        onTranslatedText(msg.text, msg.lang);
        break;

      case "peer-left":
        onStatusChange("peer-left");
        break;

      case "expired":
        onStatusChange("expired");
        break;

      case "error":
        onStatusChange("error");
        break;
    }
  };

  ws.onclose = () => onStatusChange("disconnected");
  ws.onerror = () => onStatusChange("error");

  async function addLocalAudio() {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    stream.getTracks().forEach((track) => pc.addTrack(track, stream));
    return stream;
  }

  function sendTranslatedText(text: string, lang: string) {
    ws.send(JSON.stringify({ type: "translated-text", text, lang }));
  }

  function close() {
    ws.close();
    pc.close();
  }

  return { pc, ws, addLocalAudio, sendTranslatedText, close };
}
