const { WebSocketServer } = require("ws");

const PORT = process.env.PORT || 8080;
const SESSION_TIMEOUT_MS = 2 * 60 * 1000; // expire unused sessions after 2 min

const wss = new WebSocketServer({ port: PORT });

// sessionId -> { peers: [ws, ws], createdAt: number }
const sessions = new Map();

function log(...args) {
  console.log(new Date().toISOString(), ...args);
}

wss.on("connection", (ws) => {
  ws.isAlive = true;
  ws.on("pong", () => { ws.isAlive = true; });

  ws.on("message", (raw) => {
    let msg;
    try {
      msg = JSON.parse(raw);
    } catch {
      return; // ignore malformed messages
    }

    if (msg.type === "join") {
      const existing = sessions.get(msg.sessionId);
      const peers = existing ? existing.peers : [];

      if (peers.length >= 2) {
        ws.send(JSON.stringify({ type: "error", message: "Session full" }));
        return;
      }

      peers.push(ws);
      sessions.set(msg.sessionId, { peers, createdAt: Date.now() });
      ws.sessionId = msg.sessionId;
      log("joined session", msg.sessionId, "peers:", peers.length);

      // Tell this peer whether it's first (offerer) or second (answerer)
      ws.send(JSON.stringify({ type: "role", role: peers.length === 1 ? "offerer" : "answerer" }));

      // Once two peers are present, tell both to proceed
      if (peers.length === 2) {
        peers.forEach((p) => p.send(JSON.stringify({ type: "ready" })));
      }
      return;
    }

    // Relay everything else (offer/answer/ice/translated-text) to the other peer in the session
    const session = sessions.get(ws.sessionId);
    if (!session) return;
    session.peers.forEach((peer) => {
      if (peer !== ws && peer.readyState === 1) peer.send(raw);
    });
  });

  ws.on("close", () => {
    const session = sessions.get(ws.sessionId);
    if (!session) return;
    session.peers = session.peers.filter((p) => p !== ws);
    if (session.peers.length === 0) {
      sessions.delete(ws.sessionId);
      log("session emptied and removed", ws.sessionId);
    } else {
      // let the remaining peer know the other side dropped
      session.peers.forEach((p) => p.send(JSON.stringify({ type: "peer-left" })));
    }
  });
});

// Heartbeat: drop dead connections every 30s
const heartbeat = setInterval(() => {
  wss.clients.forEach((ws) => {
    if (!ws.isAlive) return ws.terminate();
    ws.isAlive = false;
    ws.ping();
  });
}, 30000);

// Cleanup: expire sessions that never got a second peer
const cleanup = setInterval(() => {
  const now = Date.now();
  for (const [id, session] of sessions.entries()) {
    if (session.peers.length < 2 && now - session.createdAt > SESSION_TIMEOUT_MS) {
      session.peers.forEach((p) => p.send(JSON.stringify({ type: "expired" })));
      sessions.delete(id);
      log("session expired", id);
    }
  }
}, 30000);

wss.on("close", () => {
  clearInterval(heartbeat);
  clearInterval(cleanup);
});

log(`Signaling server listening on port ${PORT}`);
