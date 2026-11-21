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
    /** How long a dropped participant's seat is held for them to reconnect. 0 disables resume. */
    resumeGraceMs: toInt(env.RESUME_GRACE_MS, 20_000),
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

// ==========================================
// CONFIG VALIDATION (Phase 2 / Phase 1 condition: production settings)
// ==========================================
//
// Validates the *raw* environment so typos aren't silently replaced by
// defaults (e.g. RATE_LIMIT_PER_MINUTE="six hundred" would otherwise mean
// "off"). Never prints secret values — only names and presence.
//   errors   → the server refuses to start
//   warnings → printed at boot; with { production: true } the production
//              requirements become errors (used by `npm run check-config -- --production`)

const SECRET_KEYS = ["ELEVENLABS_API_KEY", "VITE_ELEVENLABS_API_KEY", "GEMINI_API_KEY", "DEEPL_API_KEY"];
const TRUST_PROXY_WORDS = new Set(["true", "false", "loopback", "linklocal", "uniquelocal"]);

function validateEnv(env = process.env, { production = false } = {}) {
  const errors = [];
  const warnings = [];
  const prodIssue = (msg) => (production ? errors : warnings).push(msg);

  const intVar = (name, { min = 0 } = {}) => {
    const raw = env[name];
    if (raw === undefined || raw === "") return;
    if (!/^\d+$/.test(String(raw).trim()) || Number(raw) < min) {
      errors.push(`${name} must be a whole number >= ${min} (got a non-numeric or out-of-range value)`);
    }
  };

  if (env.PORT !== undefined && env.PORT !== "" && !/^\d+$/.test(env.PORT)) errors.push("PORT must be a number");
  intVar("RATE_LIMIT_PER_MINUTE");
  intVar("MAX_TEXT_CHARS", { min: 1 });
  intVar("MAX_UPLOAD_BYTES", { min: 1 });
  intVar("MAX_WS_PAYLOAD_BYTES", { min: 1024 });
  intVar("TRANSLATE_TIMEOUT_MS", { min: 100 });
  intVar("TTS_TIMEOUT_MS", { min: 100 });
  intVar("STT_TIMEOUT_MS", { min: 100 });
  intVar("SESSION_TIMEOUT_MS", { min: 1000 });
  intVar("RESUME_GRACE_MS");
  intVar("TTS_SAMPLE_RATE", { min: 8000 });

  // ALLOWED_ORIGINS: each entry must be exactly an origin (scheme://host[:port]).
  const origins = (env.ALLOWED_ORIGINS || "").split(",").map((x) => x.trim()).filter(Boolean);
  for (const o of origins) {
    let ok = false;
    try {
      const u = new URL(o);
      ok = (u.protocol === "https:" || u.protocol === "http:") && u.origin === o.replace(/\/$/, "");
    } catch {
      ok = false;
    }
    if (!ok) errors.push(`ALLOWED_ORIGINS entry "${o}" is not an origin (expected e.g. https://app.example.com — no path, no wildcard)`);
    else if (o.startsWith("http://") && !/^http:\/\/(localhost|127\.0\.0\.1)(:\d+)?\/?$/.test(o)) {
      warnings.push(`ALLOWED_ORIGINS entry "${o}" uses http:// — browsers need https for the microphone outside localhost`);
    }
  }

  const tp = env.TRUST_PROXY;
  if (tp !== undefined && tp !== "") {
    const valid =
      TRUST_PROXY_WORDS.has(tp) ||
      /^\d+$/.test(tp) ||
      tp.split(",").every((part) => /^[0-9a-fA-F:.]+(\/\d{1,3})?$/.test(part.trim()));
    if (!valid) errors.push('TRUST_PROXY must be true/false, a hop count (e.g. 1), "loopback", or IP/CIDR list');
  }

  // Production requirements (values are deployment decisions — never guessed here).
  if (origins.length === 0) prodIssue("ALLOWED_ORIGINS is not set: any website can use this API proxy and its provider quota");
  if (!env.RATE_LIMIT_PER_MINUTE || Number(env.RATE_LIMIT_PER_MINUTE) === 0) {
    prodIssue("RATE_LIMIT_PER_MINUTE is not set: /api/* is not rate limited");
  }
  if (env.RATE_LIMIT_PER_MINUTE && Number(env.RATE_LIMIT_PER_MINUTE) > 0 && !tp) {
    warnings.push("RATE_LIMIT_PER_MINUTE is set but TRUST_PROXY is not: behind a reverse proxy every client shares one bucket");
  }
  if (!env.ELEVENLABS_API_KEY && env.VITE_ELEVENLABS_API_KEY) {
    warnings.push("ElevenLabs key is supplied as VITE_ELEVENLABS_API_KEY; rename it to ELEVENLABS_API_KEY (VITE_* names are for browser bundles)");
  }
  if (env.RESUME_GRACE_MS === "0") warnings.push("RESUME_GRACE_MS=0 disables reconnect: any network blip ends the call");

  const secrets = Object.fromEntries(SECRET_KEYS.map((k) => [k, Boolean(env[k])]));
  return { errors, warnings, secretsPresent: secrets };
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

  // sessionId -> Session. A session has up to two *seats*. A seat survives an
  // unexpected socket drop for `resumeGraceMs` and can be reclaimed with its
  // resume token; an intentional `leave` frees it at once.
  //   seat = { ws: WebSocket|null, token, role: "offerer"|"answerer", awayTimer }
  const sessions = new Map();

  const crypto = require("crypto");
  const newToken = () => crypto.randomBytes(16).toString("base64url");
  const isLobby = (id) => id.endsWith("_signal");

  function newSession() {
    return {
      seats: [],
      createdAt: Date.now(),
      /** Sockets currently connected (Phase 1 shape, still used by callers/tests). */
      get peers() {
        return this.seats.filter((s) => s.ws).map((s) => s.ws);
      },
    };
  }

  const sendJson = (ws, payload) => {
    if (ws && ws.readyState === 1) ws.send(JSON.stringify(payload));
  };

  const others = (session, seat) => session.seats.filter((s) => s !== seat && s.ws);

  function freeSeat(id, session, seat, notify = true) {
    clearTimeout(seat.awayTimer);
    session.seats = session.seats.filter((s) => s !== seat);
    if (seat.ws) {
      seat.ws.sessionId = null;
      seat.ws.seat = null;
    }
    if (session.seats.length === 0 || session.seats.every((s) => !s.ws)) {
      session.seats.forEach((s) => clearTimeout(s.awayTimer));
      sessions.delete(id);
      log("session emptied and removed");
      return;
    }
    if (notify) others(session, seat).forEach((s) => sendJson(s.ws, { type: "peer-left" }));
  }

  function seatSocket(id, session, seat, ws) {
    seat.ws = ws;
    ws.sessionId = id;
    ws.seat = seat;
  }

  function handleJoin(ws, msg) {
    if (ws.sessionId) {
      sendJson(ws, { type: "error", code: "already_joined", message: "Already in a session" });
      return;
    }
    if (typeof msg.sessionId !== "string" || !SESSION_ID_RE.test(msg.sessionId)) {
      sendJson(ws, { type: "error", code: "invalid_session", message: "Invalid session id" });
      return;
    }
    const id = msg.sessionId;
    let session = sessions.get(id);

    // --- Resume a held seat -------------------------------------------------
    if (msg.resumeToken !== undefined) {
      const seat =
        typeof msg.resumeToken === "string" && session
          ? session.seats.find((s) => s.token === msg.resumeToken)
          : null;
      if (!seat || (seat.ws && seat.ws.readyState === 1)) {
        sendJson(ws, { type: "error", code: "resume_failed", message: "That call is no longer available" });
        return;
      }
      clearTimeout(seat.awayTimer);
      seat.awayTimer = null;
      seatSocket(id, session, seat, ws);
      log("seat resumed");
      sendJson(ws, { type: "role", role: seat.role });
      sendJson(ws, { type: "joined", role: seat.role, resumeToken: seat.token, resumed: true });
      const rest = others(session, seat);
      rest.forEach((s) => sendJson(s.ws, { type: "peer-back" }));
      if (rest.length === 1) {
        [seat, ...rest].forEach((s) => sendJson(s.ws, { type: "ready", resumed: true }));
      }
      return;
    }

    // --- Fresh join ---------------------------------------------------------
    if (!session) {
      session = newSession();
      sessions.set(id, session);
    }
    // Sockets that died without a close event (no timer running) are ghosts.
    session.seats = session.seats.filter((s) => (s.ws && s.ws.readyState === 1) || s.awayTimer);

    if (session.seats.length >= 2) {
      sendJson(ws, { type: "error", code: "session_full", message: "Session full" });
      return;
    }
    const taken = new Set(session.seats.map((s) => s.role));
    const role = taken.has("offerer") ? "answerer" : "offerer";
    const seat = { ws: null, token: newToken(), role, awayTimer: null };
    session.seats.push(seat);
    seatSocket(id, session, seat, ws);
    log("joined session seats:", session.seats.length);

    sendJson(ws, { type: "role", role });
    sendJson(ws, { type: "joined", role, resumeToken: seat.token });

    if (session.seats.length === 2 && session.seats.every((s) => s.ws)) {
      session.seats.forEach((s) => sendJson(s.ws, { type: "ready" }));
    }
  }

  wss.on("connection", (ws) => {
    ws.isAlive = true;
    ws.sessionId = null;
    ws.seat = null;
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

      if (msg.type === "join") return handleJoin(ws, msg);

      const session = ws.sessionId ? sessions.get(ws.sessionId) : null;
      const seat = ws.seat;
      if (!session || !seat || !session.seats.includes(seat) || seat.ws !== ws) return;

      if (msg.type === "leave") {
        // Intentional: free the seat now so the other side learns immediately.
        freeSeat(ws.sessionId, session, seat);
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
      const text = JSON.stringify(msg);
      others(session, seat).forEach((s) => {
        if (s.ws.readyState === 1) s.ws.send(text);
      });
    });

    ws.on("close", () => {
      const id = ws.sessionId;
      const session = id ? sessions.get(id) : null;
      const seat = ws.seat;
      if (!session || !seat || seat.ws !== ws || !session.seats.includes(seat)) return;
      const remaining = others(session, seat);
      // Lobbies don't resume, and with nobody left there's nobody to resume to.
      if (isLobby(id) || remaining.length === 0 || cfg.resumeGraceMs <= 0) {
        freeSeat(id, session, seat);
        return;
      }
      // Unexpected drop: hold the seat so the same user can come back.
      seat.ws = null;
      ws.sessionId = null;
      ws.seat = null;
      remaining.forEach((s) => sendJson(s.ws, { type: "peer-away" }));
      seat.awayTimer = setTimeout(() => {
        seat.awayTimer = null;
        if (sessions.get(id) === session && session.seats.includes(seat) && !seat.ws) {
          log("held seat expired");
          freeSeat(id, session, seat);
        }
      }, cfg.resumeGraceMs);
      seat.awayTimer.unref?.();
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
      if (session.seats.length < 2 && now - session.createdAt > cfg.sessionTimeoutMs) {
        session.seats.forEach((seat) => {
          clearTimeout(seat.awayTimer);
          sendJson(seat.ws, { type: "expired" });
          if (seat.ws) {
            seat.ws.sessionId = null;
            seat.ws.seat = null;
          }
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
    for (const session of sessions.values()) session.seats.forEach((seat) => clearTimeout(seat.awayTimer));
    wss.clients.forEach((ws) => ws.terminate());
    return new Promise((resolve) => {
      wss.close(() => server.close(() => resolve()));
      server.closeAllConnections?.();
    });
  }

  return { app, server, wss, sessions, config: cfg, activeTranslator, close };
}

module.exports = { createServer, loadConfig, validateEnv, LANGUAGE_NAMES, SESSION_ID_RE };

if (require.main === module) {
  require("dotenv").config({ quiet: true });
  // Only malformed values block startup. Missing production settings are
  // warnings here (refusing to start could take a live deployment down);
  // enforce them as a deploy gate with `npm run check-config -- --production`.
  const check = validateEnv(process.env);
  if (check.errors.length) {
    defaultLog("Refusing to start — invalid configuration:");
    check.errors.forEach((e) => defaultLog(`  ERROR: ${e}`));
    process.exit(1);
  }
  const { server, config, activeTranslator } = createServer();
  server.listen(config.port, () => {
    defaultLog(`Talktual signaling + API proxy on :${config.port}`);
    defaultLog(`  translator: ${activeTranslator()}`);
    defaultLog(`  elevenlabs: ${config.elevenLabsApiKey ? `configured (${config.ttsModel})` : "NOT configured"}`);
    check.warnings.forEach((w) => defaultLog(`  WARNING: ${w}`));
  });
}
