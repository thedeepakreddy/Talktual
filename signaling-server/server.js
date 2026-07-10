const express = require("express");
const cors = require("cors");
const http = require("http");
const { WebSocketServer } = require("ws");
const multer = require("multer");

const PORT = process.env.PORT || 8080;
const SESSION_TIMEOUT_MS = 2 * 60 * 1000; // expire unused sessions after 2 min

// We also look for VITE_ELEVENLABS_API_KEY so users can just copy their frontend .env for testing
const ELEVENLABS_API_KEY = process.env.ELEVENLABS_API_KEY || process.env.VITE_ELEVENLABS_API_KEY;
const DEFAULT_VOICE_ID = '21m00Tcm4TlvDq8ikWAM'; // Rachel

const app = express();
app.use(cors());
app.use(express.json());

const upload = multer();

function log(...args) {
  console.log(new Date().toISOString(), ...args);
}

// ==========================================
// REST API ROUTES (ELEVENLABS PROXY)
// ==========================================

// TTS Proxy
app.post("/api/tts", async (req, res) => {
  const { text } = req.body;
  if (!text) return res.status(400).json({ error: "Missing text" });
  if (!ELEVENLABS_API_KEY) return res.status(500).json({ error: "Missing ElevenLabs API key on server" });

  try {
    const response = await fetch(`https://api.elevenlabs.io/v1/text-to-speech/${DEFAULT_VOICE_ID}`, {
      method: 'POST',
      headers: {
        'Accept': 'audio/mpeg',
        'Content-Type': 'application/json',
        'xi-api-key': ELEVENLABS_API_KEY
      },
      body: JSON.stringify({
        text,
        model_id: 'eleven_multilingual_v2',
        voice_settings: { stability: 0.5, similarity_boost: 0.75 }
      })
    });

    if (!response.ok) {
      const errText = await response.text();
      console.error("ElevenLabs TTS Error:", errText);
      return res.status(response.status).json({ error: "ElevenLabs API failed" });
    }

    const arrayBuffer = await response.arrayBuffer();
    const buffer = Buffer.from(arrayBuffer);
    res.set("Content-Type", "audio/mpeg");
    res.send(buffer);
  } catch (err) {
    console.error("TTS Proxy Error:", err);
    res.status(500).json({ error: "Internal server error" });
  }
});

// STT Proxy
app.post("/api/stt", upload.single('file'), async (req, res) => {
  if (!req.file) return res.status(400).json({ error: "Missing audio file" });
  if (!ELEVENLABS_API_KEY) return res.status(500).json({ error: "Missing ElevenLabs API key on server" });

  try {
    const formData = new FormData();
    const blob = new Blob([req.file.buffer], { type: req.file.mimetype || 'audio/webm' });
    formData.append('file', blob, 'recording.webm');
    formData.append('model_id', 'scribe_v1');

    const response = await fetch('https://api.elevenlabs.io/v1/speech-to-text', {
      method: 'POST',
      headers: {
        'xi-api-key': ELEVENLABS_API_KEY
      },
      body: formData
    });

    if (!response.ok) {
      const errText = await response.text();
      console.error("ElevenLabs STT Error:", errText);
      return res.status(response.status).json({ error: "ElevenLabs API failed" });
    }

    const data = await response.json();
    res.json(data);
  } catch (err) {
    console.error("STT Proxy Error:", err);
    res.status(500).json({ error: "Internal server error" });
  }
});


// ==========================================
// WEBSOCKET SIGNALING SERVER
// ==========================================

const server = http.createServer(app);
const wss = new WebSocketServer({ server });

// sessionId -> { peers: [ws, ws], createdAt: number }
const sessions = new Map();

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

server.listen(PORT, () => {
  log(`Signaling & API Proxy server listening on port ${PORT}`);
});
