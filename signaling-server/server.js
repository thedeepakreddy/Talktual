require("dotenv").config();

const express = require("express");
const cors = require("cors");
const http = require("http");
const { Readable } = require("stream");
const { WebSocketServer } = require("ws");
const multer = require("multer");

const PORT = process.env.PORT || 8080;
// Rooms are shared by QR code, which takes a moment in the real world.
const SESSION_TIMEOUT_MS = 10 * 60 * 1000;

const ELEVENLABS_API_KEY = process.env.ELEVENLABS_API_KEY || process.env.VITE_ELEVENLABS_API_KEY;
const VOICE_ID = process.env.ELEVENLABS_VOICE_ID || "21m00Tcm4TlvDq8ikWAM"; // Rachel

// flash_v2_5 is the low-latency multilingual model (~75ms to first byte).
// multilingual_v2 sounds marginally better but costs roughly a second, which
// is the difference between a conversation and a walkie-talkie with lag.
const TTS_MODEL = process.env.ELEVENLABS_TTS_MODEL || "eleven_flash_v2_5";
const STT_MODEL = process.env.ELEVENLABS_STT_MODEL || "scribe_v1";
const PCM_SAMPLE_RATE = Number(process.env.TTS_SAMPLE_RATE || 24000);

const GEMINI_API_KEY = process.env.GEMINI_API_KEY;
const GEMINI_MODEL = process.env.GEMINI_MODEL || "gemini-2.5-flash-lite";
const DEEPL_API_KEY = process.env.DEEPL_API_KEY;
const MYMEMORY_EMAIL = process.env.MYMEMORY_EMAIL;

const app = express();
app.use(
  cors({
    exposedHeaders: ["X-Audio-Format", "X-Sample-Rate", "X-Latency-Ms", "X-Provider"],
  })
);
app.use(express.json({ limit: "256kb" }));

const upload = multer({ limits: { fileSize: 25 * 1024 * 1024 } });

function log(...args) {
  console.log(new Date().toISOString(), ...args);
}

function activeTranslator() {
  if (GEMINI_API_KEY) return "gemini";
  if (DEEPL_API_KEY) return "deepl";
  return "mymemory";
}

const LANGUAGE_NAMES = {
  en: "English",
  es: "Spanish",
  de: "German",
  fa: "Persian (Farsi)",
  hu: "Hungarian",
  fr: "French",
  it: "Italian",
  pt: "Portuguese",
  hi: "Hindi",
  te: "Telugu",
  ta: "Tamil",
  ja: "Japanese",
  ko: "Korean",
  zh: "Chinese",
  ar: "Arabic",
  ru: "Russian",
  nl: "Dutch",
  pl: "Polish",
  tr: "Turkish",
  uk: "Ukrainian",
  he: "Hebrew",
  id: "Indonesian",
  vi: "Vietnamese",
  th: "Thai",
  sv: "Swedish",
  da: "Danish",
  no: "Norwegian",
  fi: "Finnish",
  cs: "Czech",
  el: "Greek",
  ro: "Romanian",
};

const iso2 = (lang) => String(lang || "").split("-")[0].toLowerCase();
const languageName = (lang) => LANGUAGE_NAMES[iso2(lang)] || iso2(lang);

// ==========================================
// CAPABILITIES
// ==========================================

app.get("/api/capabilities", (_req, res) => {
  const translator = activeTranslator();
  res.json({
    tts: Boolean(ELEVENLABS_API_KEY),
    stt: Boolean(ELEVENLABS_API_KEY),
    translator,
    // MyMemory allows 5k characters a day without an email. Translating
    // interim text would burn through that in one conversation, so live
    // captions are only offered when a real provider is configured.
    liveCaptions: translator !== "mymemory",
  });
});

app.get("/health", (_req, res) => res.json({ ok: true }));

// ==========================================
// TRANSLATION
// ==========================================

const translationCache = new Map();
const MAX_TRANSLATION_CACHE = 2000;

function cacheGet(key) {
  return translationCache.get(key);
}

function cacheSet(key, value) {
  if (translationCache.size >= MAX_TRANSLATION_CACHE) {
    translationCache.delete(translationCache.keys().next().value);
  }
  translationCache.set(key, value);
}

async function translateWithGemini(text, from, to) {
  const body = {
    contents: [
      {
        role: "user",
        parts: [
          {
            text:
              `Translate the ${languageName(from)} text below into ${languageName(to)}.\n` +
              `It is one turn of a live spoken conversation, so keep it colloquial and keep the speaker's tone.\n` +
              `Reply with the translation only — no quotes, no notes, no alternatives.\n\n` +
              text,
          },
        ],
      },
    ],
    generationConfig: { temperature: 0, maxOutputTokens: 512 },
  };
  // Thinking adds latency we cannot afford on a per-utterance path.
  if (GEMINI_MODEL.includes("2.5")) {
    body.generationConfig.thinkingConfig = { thinkingBudget: 0 };
  }

  const res = await fetch(
    `https://generativelanguage.googleapis.com/v1beta/models/${GEMINI_MODEL}:generateContent`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json", "x-goog-api-key": GEMINI_API_KEY },
      body: JSON.stringify(body),
    }
  );

  if (!res.ok) throw new Error(`Gemini ${res.status}: ${await res.text()}`);
  const data = await res.json();
  const out = data?.candidates?.[0]?.content?.parts?.map((p) => p.text).join("").trim();
  if (!out) throw new Error("Gemini returned no translation");
  return out;
}

async function translateWithDeepL(text, from, to) {
  const host = DEEPL_API_KEY.endsWith(":fx") ? "api-free.deepl.com" : "api.deepl.com";
  const res = await fetch(`https://${host}/v2/translate`, {
    method: "POST",
    headers: {
      Authorization: `DeepL-Auth-Key ${DEEPL_API_KEY}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      text: [text],
      source_lang: iso2(from).toUpperCase(),
      target_lang: iso2(to).toUpperCase(),
    }),
  });
  if (!res.ok) throw new Error(`DeepL ${res.status}: ${await res.text()}`);
  const data = await res.json();
  const out = data?.translations?.[0]?.text;
  if (!out) throw new Error("DeepL returned no translation");
  return out;
}

async function translateWithMyMemory(text, from, to) {
  const params = new URLSearchParams({ q: text, langpair: `${iso2(from)}|${iso2(to)}` });
  if (MYMEMORY_EMAIL) params.set("de", MYMEMORY_EMAIL);

  const res = await fetch(`https://api.mymemory.translated.net/get?${params}`);
  if (!res.ok) throw new Error(`MyMemory ${res.status}`);
  const data = await res.json();
  if (data.responseStatus !== 200) throw new Error(data.responseDetails || "MyMemory failed");
  return data.responseData.translatedText;
}

app.post("/api/translate", async (req, res) => {
  const { text, from, to } = req.body || {};
  if (!text) return res.status(400).json({ error: "Missing text" });
  if (!to) return res.status(400).json({ error: "Missing target language" });

  if (iso2(from) === iso2(to)) return res.json({ text, provider: "passthrough" });

  const key = `${iso2(from)}>${iso2(to)}:${text}`;
  const hit = cacheGet(key);
  if (hit) {
    res.set("X-Latency-Ms", "0");
    res.set("X-Provider", "cache");
    return res.json({ text: hit, provider: "cache" });
  }

  const started = Date.now();
  const provider = activeTranslator();

  try {
    let translated;
    if (provider === "gemini") translated = await translateWithGemini(text, from, to);
    else if (provider === "deepl") translated = await translateWithDeepL(text, from, to);
    else translated = await translateWithMyMemory(text, from, to);

    cacheSet(key, translated);
    const ms = Date.now() - started;
    res.set("X-Latency-Ms", String(ms));
    res.set("X-Provider", provider);
    log(`translate ${iso2(from)}>${iso2(to)} via ${provider} ${ms}ms`);
    res.json({ text: translated, provider, latencyMs: ms });
  } catch (err) {
    log("translate failed via", provider, "-", err.message);
    // Try the keyless provider before giving up, unless that's what just failed.
    if (provider !== "mymemory") {
      try {
        const translated = await translateWithMyMemory(text, from, to);
        cacheSet(key, translated);
        res.set("X-Provider", "mymemory");
        return res.json({ text: translated, provider: "mymemory", degraded: true });
      } catch (fallbackErr) {
        log("mymemory fallback failed:", fallbackErr.message);
      }
    }
    // Returning the source text keeps the call alive rather than dropping a turn.
    res.json({ text, provider: "none", error: err.message });
  }
});

// ==========================================
// TEXT TO SPEECH (streamed)
// ==========================================

async function requestElevenTts(text, lang, outputFormat) {
  const params = new URLSearchParams({
    optimize_streaming_latency: "3",
    output_format: outputFormat,
  });

  const body = {
    text,
    model_id: TTS_MODEL,
    voice_settings: { stability: 0.4, similarity_boost: 0.75, speed: 1.0 },
  };
  // flash/turbo v2.5 accept an explicit language, which stops the model from
  // guessing wrong on short utterances.
  if (/flash|turbo/.test(TTS_MODEL) && lang) body.language_code = iso2(lang);

  return fetch(`https://api.elevenlabs.io/v1/text-to-speech/${VOICE_ID}/stream?${params}`, {
    method: "POST",
    headers: {
      Accept: "*/*",
      "Content-Type": "application/json",
      "xi-api-key": ELEVENLABS_API_KEY,
    },
    body: JSON.stringify(body),
  });
}

app.post("/api/tts", async (req, res) => {
  const { text, lang } = req.body || {};
  if (!text) return res.status(400).json({ error: "Missing text" });
  if (!ELEVENLABS_API_KEY) {
    return res.status(503).json({ error: "No ElevenLabs key configured", fallback: "browser" });
  }

  const started = Date.now();

  try {
    // Raw PCM lets the browser start playing on the first chunk without having
    // to parse a container. Not every account tier allows every PCM rate, so
    // fall back to MP3 if the format is rejected.
    let upstream = await requestElevenTts(text, lang, `pcm_${PCM_SAMPLE_RATE}`);
    let format = "pcm";

    if (!upstream.ok && upstream.status >= 400 && upstream.status < 500) {
      const detail = await upstream.text();
      log(`TTS pcm rejected (${upstream.status}), retrying as mp3:`, detail.slice(0, 200));
      upstream = await requestElevenTts(text, lang, "mp3_44100_128");
      format = "mp3";
    }

    if (!upstream.ok || !upstream.body) {
      const detail = await upstream.text().catch(() => "");
      log("ElevenLabs TTS error:", upstream.status, detail.slice(0, 300));
      return res.status(upstream.status).json({ error: "TTS failed", fallback: "browser" });
    }

    res.set("X-Audio-Format", format);
    res.set("X-Latency-Ms", String(Date.now() - started));
    if (format === "pcm") {
      res.set("Content-Type", "audio/pcm");
      res.set("X-Sample-Rate", String(PCM_SAMPLE_RATE));
    } else {
      res.set("Content-Type", "audio/mpeg");
    }
    res.set("Cache-Control", "no-store");

    const stream = Readable.fromWeb(upstream.body);
    stream.on("error", (err) => {
      log("TTS stream error:", err.message);
      res.destroy(err);
    });
    res.on("close", () => stream.destroy());
    stream.pipe(res);
  } catch (err) {
    log("TTS proxy error:", err.message);
    if (!res.headersSent) res.status(500).json({ error: "TTS proxy error", fallback: "browser" });
  }
});

// ==========================================
// SPEECH TO TEXT (fallback path for browsers without Web Speech)
// ==========================================

app.post("/api/stt", upload.single("file"), async (req, res) => {
  if (!req.file) return res.status(400).json({ error: "Missing audio file" });
  if (!ELEVENLABS_API_KEY) return res.status(503).json({ error: "No ElevenLabs key configured" });

  const started = Date.now();

  try {
    const form = new FormData();
    const blob = new Blob([req.file.buffer], { type: req.file.mimetype || "audio/webm" });
    form.append("file", blob, req.file.originalname || "push.webm");
    form.append("model_id", STT_MODEL);
    // A language hint measurably improves accuracy on short utterances.
    if (req.body.language_code) form.append("language_code", iso2(req.body.language_code));

    const upstream = await fetch("https://api.elevenlabs.io/v1/speech-to-text", {
      method: "POST",
      headers: { "xi-api-key": ELEVENLABS_API_KEY },
      body: form,
    });

    if (!upstream.ok) {
      const detail = await upstream.text();
      log("ElevenLabs STT error:", upstream.status, detail.slice(0, 300));
      return res.status(upstream.status).json({ error: "Transcription failed" });
    }

    const data = await upstream.json();
    const ms = Date.now() - started;
    log(`stt ${req.file.size}B ${ms}ms`);
    res.set("X-Latency-Ms", String(ms));
    res.json(data);
  } catch (err) {
    log("STT proxy error:", err.message);
    res.status(500).json({ error: "STT proxy error" });
  }
});

// ==========================================
// WEBSOCKET SIGNALING
// ==========================================

const server = http.createServer(app);
const wss = new WebSocketServer({ server });

// sessionId -> { peers: [ws, ws], createdAt: number }
const sessions = new Map();

wss.on("connection", (ws) => {
  ws.isAlive = true;
  ws.on("pong", () => {
    ws.isAlive = true;
  });

  ws.on("message", (raw) => {
    let msg;
    try {
      msg = JSON.parse(raw);
    } catch {
      return;
    }

    if (msg.type === "join") {
      const existing = sessions.get(msg.sessionId);
      const peers = existing ? existing.peers : [];

      if (peers.length >= 2) {
        ws.send(JSON.stringify({ type: "error", message: "Session full" }));
        return;
      }

      peers.push(ws);
      sessions.set(msg.sessionId, { peers, createdAt: existing?.createdAt ?? Date.now() });
      ws.sessionId = msg.sessionId;
      log("joined session", msg.sessionId, "peers:", peers.length);

      ws.send(JSON.stringify({ type: "role", role: peers.length === 1 ? "offerer" : "answerer" }));

      if (peers.length === 2) {
        peers.forEach((p) => p.send(JSON.stringify({ type: "ready" })));
      }
      return;
    }

    // Relay signaling (and any app messages sent before the data channel opens)
    // to the other peer in the session.
    //
    // Re-serialize rather than forwarding `raw`: ws hands us a Buffer, and
    // sending a Buffer produces a *binary* frame. The browser then surfaces it
    // as a Blob, JSON.parse throws, and every relayed message — offers,
    // answers, ICE candidates — is silently swallowed.
    const session = sessions.get(ws.sessionId);
    if (!session) return;
    const text = JSON.stringify(msg);
    session.peers.forEach((peer) => {
      if (peer !== ws && peer.readyState === 1) peer.send(text);
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
      session.peers.forEach((p) => p.send(JSON.stringify({ type: "peer-left" })));
    }
  });
});

const heartbeat = setInterval(() => {
  wss.clients.forEach((ws) => {
    if (!ws.isAlive) return ws.terminate();
    ws.isAlive = false;
    ws.ping();
  });
}, 30000);

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
  log(`Talktual signaling + API proxy on :${PORT}`);
  log(`  translator: ${activeTranslator()}`);
  log(`  elevenlabs: ${ELEVENLABS_API_KEY ? `configured (${TTS_MODEL})` : "NOT configured"}`);
});
