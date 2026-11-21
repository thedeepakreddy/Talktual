// Talktual signaling server + API proxy.
//
// One process serves the WebSocket signaling endpoint and the HTTP proxy for
// translation, TTS and STT, so provider keys never reach the browser.
//
// The module exports a factory (createServer) so tests can build an isolated
// instance with a stubbed `fetch` and explicit config. Running the file
// directly (`node server.js`) loads .env and listens, exactly as before.

const express = require("express");
const cors = require("cors");
const http = require("http");
const { Readable } = require("stream");
const { WebSocketServer } = require("ws");
const multer = require("multer");

// ==========================================
// CONFIG
// ==========================================

function toInt(value, fallback) {
  const n = Number(value);
  return Number.isFinite(n) && value !== undefined && value !== "" ? n : fallback;
}

function loadConfig(env = process.env) {
  const elevenKey = env.ELEVENLABS_API_KEY || env.VITE_ELEVENLABS_API_KEY;
  return {
    port: env.PORT || 8080,
    // Rooms are shared by QR code, which takes a moment in the real world.
    sessionTimeoutMs: toInt(env.SESSION_TIMEOUT_MS, 10 * 60 * 1000),
    elevenLabsApiKey: elevenKey,
    elevenLabsKeyFromViteVar: !env.ELEVENLABS_API_KEY && Boolean(env.VITE_ELEVENLABS_API_KEY),
    voiceId: env.ELEVENLABS_VOICE_ID || "21m00Tcm4TlvDq8ikWAM", // Rachel
    // flash_v2_5 is the low-latency multilingual model (~75ms to first byte).
    // multilingual_v2 sounds marginally better but costs roughly a second, which
    // is the difference between a conversation and a walkie-talkie with lag.
    ttsModel: env.ELEVENLABS_TTS_MODEL || "eleven_flash_v2_5",
    sttModel: env.ELEVENLABS_STT_MODEL || "scribe_v1",
    pcmSampleRate: toInt(env.TTS_SAMPLE_RATE, 24000),
    geminiApiKey: env.GEMINI_API_KEY,
    geminiModel: env.GEMINI_MODEL || "gemini-2.5-flash-lite",
    deeplApiKey: env.DEEPL_API_KEY,
    myMemoryEmail: env.MYMEMORY_EMAIL,

    // --- Hardening (Phase 1) ---
    /** Longest text accepted by /api/translate and /api/tts, in characters. */
    maxTextChars: toInt(env.MAX_TEXT_CHARS, 2000),
    /** Largest STT upload, in bytes. A 60s opus push is well under 1 MB. */
    maxUploadBytes: toInt(env.MAX_UPLOAD_BYTES, 10 * 1024 * 1024),
    /** Largest WebSocket frame. SDP offers are a few KB; captions are tiny. */
    maxWsPayloadBytes: toInt(env.MAX_WS_PAYLOAD_BYTES, 64 * 1024),
    /** Upstream provider budgets. Without these a stalled provider hangs the turn. */
    translateTimeoutMs: toInt(env.TRANSLATE_TIMEOUT_MS, 8000),
    ttsTimeoutMs: toInt(env.TTS_TIMEOUT_MS, 10000),
    sttTimeoutMs: toInt(env.STT_TIMEOUT_MS, 30000),
    /** Comma-separated browser origins allowed to use the API and signaling. Unset = any. */
    allowedOrigins: (env.ALLOWED_ORIGINS || "")
      .split(",")
      .map((s) => s.trim().replace(/\/$/, ""))
      .filter(Boolean),
    /** Per-IP request budget for /api/* per minute. 0 disables the limiter. */
    rateLimitPerMinute: toInt(env.RATE_LIMIT_PER_MINUTE, 0),
    /** Express "trust proxy" setting; set to 1 behind Render/Heroku-style proxies. */
    trustProxy: env.TRUST_PROXY === undefined ? false : env.TRUST_PROXY === "true" ? true : toInt(env.TRUST_PROXY, env.TRUST_PROXY),
    heartbeatMs: toInt(env.HEARTBEAT_MS, 30000),
    cleanupMs: toInt(env.CLEANUP_MS, 30000),
  };
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
const isSupportedLanguage = (lang) => typeof lang === "string" && Object.hasOwn(LANGUAGE_NAMES, iso2(lang));
const languageName = (lang) => LANGUAGE_NAMES[iso2(lang)] || iso2(lang);

/** Session ids are room codes: 4-12 uppercase alphanumerics, optionally with the lobby suffix. */
const SESSION_ID_RE = /^[A-Z0-9]{4,12}(_signal)?$/;
/** Only these signaling types are forwarded to the other peer. */
const RELAY_TYPES = new Set(["offer", "answer", "ice", "app"]);

function defaultLog(...args) {
  console.log(new Date().toISOString(), ...args);
}

// ==========================================
// FACTORY
// ==========================================

function createServer(overrides = {}) {
  const { fetch: fetchOverride, log: logOverride, ...configOverrides } = overrides;
  const cfg = { ...loadConfig(), ...configOverrides };
  const fetchImpl = fetchOverride || globalThis.fetch;
  const log = logOverride || defaultLog;

  function activeTranslator() {
    if (cfg.geminiApiKey) return "gemini";
    if (cfg.deeplApiKey) return "deepl";
    return "mymemory";
  }

  const originAllowed = (origin) =>
    cfg.allowedOrigins.length === 0 || (typeof origin === "string" && cfg.allowedOrigins.includes(origin));

  const app = express();
  app.disable("x-powered-by");
  app.set("trust proxy", cfg.trustProxy);

  app.use((_req, res, next) => {
    res.set("X-Content-Type-Options", "nosniff");
    res.set("Referrer-Policy", "no-referrer");
    next();
  });

  app.use(
    cors({
      origin: cfg.allowedOrigins.length ? (origin, cb) => cb(null, !origin || originAllowed(origin)) : "*",
      exposedHeaders: ["X-Audio-Format", "X-Sample-Rate", "X-Latency-Ms", "X-Provider"],
    })
  );

  // Browsers always send Origin on cross-origin POSTs; when an allowlist is
  // configured, refuse API use from other sites outright (CORS alone only
  // stops the page from *reading* the response, not the spend).
  app.use("/api", (req, res, next) => {
    const origin = req.get("origin");
    if (origin && !originAllowed(origin)) return res.status(403).json({ error: "origin_not_allowed" });
    next();
  });

  // --- Optional per-IP rate limiter for the paid provider endpoints ---------
  const buckets = new Map();
  if (cfg.rateLimitPerMinute > 0) {
    app.use("/api", (req, res, next) => {
      if (req.method === "OPTIONS" || req.path === "/capabilities") return next();
      const now = Date.now();
      const key = req.ip || "unknown";
      let b = buckets.get(key);
      if (!b || now >= b.resetAt) {
        b = { count: 0, resetAt: now + 60_000 };
        buckets.set(key, b);
      }
      b.count++;
      if (b.count > cfg.rateLimitPerMinute) {
        res.set("Retry-After", String(Math.ceil((b.resetAt - now) / 1000)));
        return res.status(429).json({ error: "rate_limited" });
      }
      next();
    });
  }

  app.use(express.json({ limit: "64kb" }));

  const upload = multer({ limits: { fileSize: cfg.maxUploadBytes, files: 1, fields: 4 } });

  // ==========================================
  // CAPABILITIES
  // ==========================================

  app.get("/api/capabilities", (_req, res) => {
    const translator = activeTranslator();
    res.json({
      tts: Boolean(cfg.elevenLabsApiKey),
      stt: Boolean(cfg.elevenLabsApiKey),
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

  function cacheSet(key, value) {
    if (translationCache.size >= MAX_TRANSLATION_CACHE) {
      translationCache.delete(translationCache.keys().next().value);
    }
    translationCache.set(key, value);
  }

  const timeoutSignal = (ms) => AbortSignal.timeout(ms);

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
    if (cfg.geminiModel.includes("2.5")) {
      body.generationConfig.thinkingConfig = { thinkingBudget: 0 };
    }

    const res = await fetchImpl(
      `https://generativelanguage.googleapis.com/v1beta/models/${cfg.geminiModel}:generateContent`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json", "x-goog-api-key": cfg.geminiApiKey },
        body: JSON.stringify(body),
        signal: timeoutSignal(cfg.translateTimeoutMs),
      }
    );

    if (!res.ok) throw new Error(`Gemini ${res.status}: ${(await res.text()).slice(0, 200)}`);
    const data = await res.json();
    const out = data?.candidates?.[0]?.content?.parts?.map((p) => p.text).join("").trim();
    if (!out) throw new Error("Gemini returned no translation");
    return out;
  }

  async function translateWithDeepL(text, from, to) {
    const host = cfg.deeplApiKey.endsWith(":fx") ? "api-free.deepl.com" : "api.deepl.com";
    const res = await fetchImpl(`https://${host}/v2/translate`, {
      method: "POST",
      headers: {
        Authorization: `DeepL-Auth-Key ${cfg.deeplApiKey}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        text: [text],
        source_lang: iso2(from).toUpperCase(),
        target_lang: iso2(to).toUpperCase(),
      }),
      signal: timeoutSignal(cfg.translateTimeoutMs),
    });
    if (!res.ok) throw new Error(`DeepL ${res.status}: ${(await res.text()).slice(0, 200)}`);
    const data = await res.json();
    const out = data?.translations?.[0]?.text;
    if (!out) throw new Error("DeepL returned no translation");
    return out;
  }

  async function translateWithMyMemory(text, from, to) {
    const params = new URLSearchParams({ q: text, langpair: `${iso2(from)}|${iso2(to)}` });
    if (cfg.myMemoryEmail) params.set("de", cfg.myMemoryEmail);

    const res = await fetchImpl(`https://api.mymemory.translated.net/get?${params}`, {
      signal: timeoutSignal(cfg.translateTimeoutMs),
    });
    if (!res.ok) throw new Error(`MyMemory ${res.status}`);
    const data = await res.json();
    if (Number(data?.responseStatus) !== 200) throw new Error(data?.responseDetails || "MyMemory failed");
    const out = data?.responseData?.translatedText;
    if (typeof out !== "string" || !out.trim()) throw new Error("MyMemory returned no translation");
    return out;
  }

  function validateText(text, res) {
    if (typeof text !== "string" || !text.trim()) {
      res.status(400).json({ error: "Missing text" });
      return false;
    }
    if (text.length > cfg.maxTextChars) {
      res.status(413).json({ error: "text_too_long", maxChars: cfg.maxTextChars });
      return false;
    }
    return true;
  }

  app.post("/api/translate", async (req, res) => {
    const { text, from, to } = req.body || {};
    if (!validateText(text, res)) return;
    if (!to) return res.status(400).json({ error: "Missing target language" });
    if (!isSupportedLanguage(to) || !isSupportedLanguage(from)) {
      return res.status(400).json({ error: "unsupported_language" });
    }

    if (iso2(from) === iso2(to)) return res.json({ text, provider: "passthrough" });

    const key = `${iso2(from)}>${iso2(to)}:${text}`;
    const hit = translationCache.get(key);
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
      // Fail explicitly. The client decides how to keep the call alive (it
      // sends the original text, clearly marked as untranslated); the server
      // must not dress source text up as a successful translation.
      const timedOut = err?.name === "TimeoutError" || err?.name === "AbortError";
      res.status(timedOut ? 504 : 502).json({ error: timedOut ? "translation_timeout" : "translation_failed" });
    }
  });

  // ==========================================
  // TEXT TO SPEECH (streamed)
  // ==========================================

  async function requestElevenTts(text, lang, outputFormat, signal) {
    const params = new URLSearchParams({
      optimize_streaming_latency: "3",
      output_format: outputFormat,
    });

    const body = {
      text,
      model_id: cfg.ttsModel,
      voice_settings: { stability: 0.4, similarity_boost: 0.75, speed: 1.0 },
    };
    // flash/turbo v2.5 accept an explicit language, which stops the model from
    // guessing wrong on short utterances.
    if (/flash|turbo/.test(cfg.ttsModel) && isSupportedLanguage(lang)) body.language_code = iso2(lang);

    return fetchImpl(`https://api.elevenlabs.io/v1/text-to-speech/${cfg.voiceId}/stream?${params}`, {
      method: "POST",
      headers: {
        Accept: "*/*",
        "Content-Type": "application/json",
        "xi-api-key": cfg.elevenLabsApiKey,
      },
      body: JSON.stringify(body),
      signal,
    });
  }

  app.post("/api/tts", async (req, res) => {
    const { text, lang } = req.body || {};
    if (!validateText(text, res)) return;
    if (!cfg.elevenLabsApiKey) {
      return res.status(503).json({ error: "No ElevenLabs key configured", fallback: "browser" });
    }

    const started = Date.now();
    // Cancels the upstream request if the browser goes away (barge-in) or the
    // provider never answers. The timeout only covers time-to-headers.
    const upstreamAbort = new AbortController();
    const headerTimer = setTimeout(() => upstreamAbort.abort(new Error("TTS upstream timeout")), cfg.ttsTimeoutMs);
    res.on("close", () => upstreamAbort.abort());

    try {
      // Raw PCM lets the browser start playing on the first chunk without having
      // to parse a container. Not every account tier allows every PCM rate, so
      // fall back to MP3 if the format is rejected.
      let upstream = await requestElevenTts(text, lang, `pcm_${cfg.pcmSampleRate}`, upstreamAbort.signal);
      let format = "pcm";

      if (!upstream.ok && upstream.status >= 400 && upstream.status < 500) {
        const detail = await upstream.text();
        log(`TTS pcm rejected (${upstream.status}), retrying as mp3:`, detail.slice(0, 200));
        upstream = await requestElevenTts(text, lang, "mp3_44100_128", upstreamAbort.signal);
        format = "mp3";
      }
      clearTimeout(headerTimer);

      if (!upstream.ok || !upstream.body) {
        const detail = await upstream.text().catch(() => "");
        log("ElevenLabs TTS error:", upstream.status, detail.slice(0, 300));
        return res.status(502).json({ error: "TTS failed", fallback: "browser" });
      }

      res.set("X-Audio-Format", format);
      res.set("X-Latency-Ms", String(Date.now() - started));
      if (format === "pcm") {
        res.set("Content-Type", "audio/pcm");
        res.set("X-Sample-Rate", String(cfg.pcmSampleRate));
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
      clearTimeout(headerTimer);
      log("TTS proxy error:", err.message);
      if (!res.headersSent && !res.destroyed) {
        res.status(504).json({ error: "TTS proxy error", fallback: "browser" });
      }
    }
  });

  // ==========================================
  // SPEECH TO TEXT (fallback path for browsers without Web Speech)
  // ==========================================

  app.post("/api/stt", upload.single("file"), async (req, res) => {
    if (!req.file) return res.status(400).json({ error: "Missing audio file" });
    if (!cfg.elevenLabsApiKey) return res.status(503).json({ error: "No ElevenLabs key configured" });

    const started = Date.now();

    try {
      const form = new FormData();
      const mimetype = /^audio\/[\w.+-]+(;.*)?$/.test(req.file.mimetype || "") ? req.file.mimetype : "audio/webm";
      const blob = new Blob([req.file.buffer], { type: mimetype });
      form.append("file", blob, "push.webm");
      form.append("model_id", cfg.sttModel);
      // A language hint measurably improves accuracy on short utterances.
      const hint = req.body?.language_code;
      if (isSupportedLanguage(hint)) form.append("language_code", iso2(hint));

      const upstream = await fetchImpl("https://api.elevenlabs.io/v1/speech-to-text", {
        method: "POST",
        headers: { "xi-api-key": cfg.elevenLabsApiKey },
        body: form,
        signal: timeoutSignal(cfg.sttTimeoutMs),
      });

      if (!upstream.ok) {
        const detail = await upstream.text();
        log("ElevenLabs STT error:", upstream.status, detail.slice(0, 300));
        return res.status(502).json({ error: "Transcription failed" });
      }

      const data = await upstream.json();
      const ms = Date.now() - started;
      log(`stt ${req.file.size}B ${ms}ms`);
      res.set("X-Latency-Ms", String(ms));
      // Only the transcript is needed; don't forward provider internals.
      res.json({ text: typeof data?.text === "string" ? data.text : "", language_code: data?.language_code });
    } catch (err) {
      log("STT proxy error:", err.message);
      res.status(502).json({ error: "STT proxy error" });
    }
  });

  // Consistent JSON errors. Express's default handler renders an HTML page
  // with a stack trace (and absolute file paths) outside production.
  // eslint-disable-next-line no-unused-vars
  app.use((err, _req, res, _next) => {
    if (res.headersSent) return;
    if (err?.type === "entity.parse.failed") return res.status(400).json({ error: "invalid_json" });
    if (err?.type === "entity.too.large") return res.status(413).json({ error: "payload_too_large" });
    if (err?.code === "LIMIT_FILE_SIZE") return res.status(413).json({ error: "file_too_large" });
    if (err instanceof multer.MulterError) return res.status(400).json({ error: "invalid_upload" });
    log("unhandled request error:", err?.message);
    res.status(500).json({ error: "internal_error" });
  });

  // ==========================================
  // WEBSOCKET SIGNALING
  // ==========================================

  const server = http.createServer(app);
  const wss = new WebSocketServer({
    server,
    maxPayload: cfg.maxWsPayloadBytes,
    verifyClient: (info) => originAllowed(info.origin || info.req.headers.origin),
  });

  // sessionId -> { peers: [ws, ws], createdAt: number }
  const sessions = new Map();

  const sendJson = (ws, payload) => {
    if (ws.readyState === 1) ws.send(JSON.stringify(payload));
  };

  wss.on("connection", (ws) => {
    ws.isAlive = true;
    ws.sessionId = null;
    ws.on("pong", () => {
      ws.isAlive = true;
    });
    // Without a listener, a protocol error (invalid UTF-8, oversized frame,
    // bad opcode) is an unhandled 'error' event and takes the whole process —
    // and every call on it — down. ws already closes the socket for us.
    ws.on("error", (err) => log("ws client error:", err.code || err.message));

    ws.on("message", (raw, isBinary) => {
      if (isBinary) return;
      let msg;
      try {
        msg = JSON.parse(raw);
      } catch {
        return;
      }
      if (!msg || typeof msg !== "object" || typeof msg.type !== "string") return;

      if (msg.type === "join") {
        if (ws.sessionId) {
          sendJson(ws, { type: "error", code: "already_joined", message: "Already in a session" });
          return;
        }
        if (typeof msg.sessionId !== "string" || !SESSION_ID_RE.test(msg.sessionId)) {
          sendJson(ws, { type: "error", code: "invalid_session", message: "Invalid session id" });
          return;
        }
        const existing = sessions.get(msg.sessionId);
        // Drop peers whose sockets are already gone so a ghost can't hold a seat.
        const peers = existing ? existing.peers.filter((p) => p.readyState === 1) : [];

        if (peers.length >= 2) {
          sendJson(ws, { type: "error", code: "session_full", message: "Session full" });
          return;
        }

        peers.push(ws);
        sessions.set(msg.sessionId, { peers, createdAt: existing?.createdAt ?? Date.now() });
        ws.sessionId = msg.sessionId;
        log("joined session peers:", peers.length);

        sendJson(ws, { type: "role", role: peers.length === 1 ? "offerer" : "answerer" });

        if (peers.length === 2) {
          peers.forEach((p) => sendJson(p, { type: "ready" }));
        }
        return;
      }

      if (!RELAY_TYPES.has(msg.type)) return;

      // Relay signaling (and any app messages sent before the data channel opens)
      // to the other peer in the session.
      //
      // Re-serialize rather than forwarding `raw`: ws hands us a Buffer, and
      // sending a Buffer produces a *binary* frame. The browser then surfaces it
      // as a Blob, JSON.parse throws, and every relayed message — offers,
      // answers, ICE candidates — is silently swallowed.
      const session = ws.sessionId ? sessions.get(ws.sessionId) : null;
      if (!session || !session.peers.includes(ws)) return;
      const text = JSON.stringify(msg);
      session.peers.forEach((peer) => {
        if (peer !== ws && peer.readyState === 1) peer.send(text);
      });
    });

    ws.on("close", () => {
      const session = ws.sessionId ? sessions.get(ws.sessionId) : null;
      if (!session || !session.peers.includes(ws)) return;
      session.peers = session.peers.filter((p) => p !== ws);
      if (session.peers.length === 0) {
        sessions.delete(ws.sessionId);
        log("session emptied and removed");
      } else {
        session.peers.forEach((p) => sendJson(p, { type: "peer-left" }));
      }
    });
  });

  const heartbeat = setInterval(() => {
    wss.clients.forEach((ws) => {
      if (!ws.isAlive) return ws.terminate();
      ws.isAlive = false;
      ws.ping();
    });
  }, cfg.heartbeatMs);

  const cleanup = setInterval(() => {
    const now = Date.now();
    for (const [id, session] of sessions.entries()) {
      if (session.peers.length < 2 && now - session.createdAt > cfg.sessionTimeoutMs) {
        session.peers.forEach((p) => {
          sendJson(p, { type: "expired" });
          p.sessionId = null;
        });
        sessions.delete(id);
        log("session expired");
      }
    }
    for (const [ip, b] of buckets) if (now >= b.resetAt) buckets.delete(ip);
  }, cfg.cleanupMs);
  heartbeat.unref?.();
  cleanup.unref?.();

  function close() {
    clearInterval(heartbeat);
    clearInterval(cleanup);
    wss.clients.forEach((ws) => ws.terminate());
    return new Promise((resolve) => {
      wss.close(() => server.close(() => resolve()));
      server.closeAllConnections?.();
    });
  }

  return { app, server, wss, sessions, config: cfg, activeTranslator, close };
}

module.exports = { createServer, loadConfig, LANGUAGE_NAMES, SESSION_ID_RE };

if (require.main === module) {
  require("dotenv").config({ quiet: true });
  const { server, config, activeTranslator } = createServer();
  server.listen(config.port, () => {
    defaultLog(`Talktual signaling + API proxy on :${config.port}`);
    defaultLog(`  translator: ${activeTranslator()}`);
    defaultLog(`  elevenlabs: ${config.elevenLabsApiKey ? `configured (${config.ttsModel})` : "NOT configured"}`);
    if (config.elevenLabsKeyFromViteVar) {
      defaultLog("  WARNING: ElevenLabs key read from VITE_ELEVENLABS_API_KEY. Rename it to ELEVENLABS_API_KEY;");
      defaultLog("           VITE_* variables are meant for the browser bundle and must never hold secrets.");
    }
    if (config.allowedOrigins.length === 0) {
      defaultLog("  WARNING: ALLOWED_ORIGINS is unset — any website can use this API proxy and its provider quota.");
    }
    if (!config.rateLimitPerMinute) {
      defaultLog("  WARNING: RATE_LIMIT_PER_MINUTE is unset — /api/* is not rate limited.");
    }
  });
}
