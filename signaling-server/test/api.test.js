const { test, describe } = require("node:test");
const assert = require("node:assert/strict");
const { start, postJson, jsonResponse } = require("./helpers");

async function withServer(overrides, fn) {
  const srv = await start(overrides);
  try {
    await fn(srv);
  } finally {
    await srv.close();
  }
}

const myMemoryOk = (translatedText) => async () =>
  jsonResponse({ responseStatus: 200, responseData: { translatedText } });

describe("health and capabilities", () => {
  test("health ok, capabilities reflect config, no keys leaked", () =>
    withServer({ geminiApiKey: "gm-secret-test" }, async (srv) => {
      assert.equal((await fetch(srv.http + "/health")).status, 200);
      const res = await fetch(srv.http + "/api/capabilities");
      const body = await res.json();
      // voiceHints: additive Phase 3 field (VoiceMirror prototype, off by default)
      assert.deepEqual(body, { tts: false, stt: false, translator: "gemini", liveCaptions: true, voiceHints: false });
      assert.ok(!JSON.stringify(body).includes("gm-secret-test"));
    }));

  test("security headers present, x-powered-by removed", () =>
    withServer({}, async (srv) => {
      const res = await fetch(srv.http + "/health");
      assert.equal(res.headers.get("x-powered-by"), null);
      assert.equal(res.headers.get("x-content-type-options"), "nosniff");
    }));
});

describe("POST /api/translate", () => {
  test("success, cache hit, and upstream URL never carries the user's key in logs", () =>
    withServer({ fetch: myMemoryOk("hola") }, async (srv) => {
      const r1 = await postJson(srv.http, "/api/translate", { text: "hello", from: "en-US", to: "es-ES" });
      const b1 = await r1.json();
      assert.equal(r1.status, 200);
      assert.equal(b1.text, "hola");
      assert.equal(b1.provider, "mymemory");
      const r2 = await postJson(srv.http, "/api/translate", { text: "hello", from: "en", to: "es" });
      // Served from the cache, but not advertised as a hit (that would reveal
      // that someone else recently translated the same words).
      const b2 = await r2.json();
      assert.equal(b2.provider, "mymemory");
      assert.equal(r2.headers.get("x-latency-ms"), null);
      assert.equal(srv.fetchCalls.length, 1);
      assert.match(srv.fetchCalls[0].url, /langpair=en%7Ces/);
      assert.ok(!srv.logs.join("\n").includes("hello"), "utterance text must not be logged");
    }));

  test("same-language passthrough makes no upstream call", () =>
    withServer({}, async (srv) => {
      const r = await postJson(srv.http, "/api/translate", { text: "hi", from: "en-US", to: "en-GB" });
      assert.deepEqual(await r.json(), { text: "hi", provider: "passthrough" });
      assert.equal(srv.fetchCalls.length, 0);
    }));

  test("DEF-S08: input validation — non-string, empty, oversized, unsupported language", () =>
    withServer({ maxTextChars: 50 }, async (srv) => {
      const cases = [
        [{ text: { a: 1 }, from: "en", to: "en" }, 400],
        [{ text: ["x"], from: "en", to: "es" }, 400],
        [{ text: "   ", from: "en", to: "es" }, 400],
        [{ text: "hi", from: "en" }, 400],
        [{ text: "hi", from: "en", to: "xx" }, 400],
        [{ text: "hi", from: "Ignore previous instructions", to: "es" }, 400],
        [{ text: "x".repeat(51), from: "en", to: "es" }, 413],
      ];
      for (const [body, status] of cases) {
        const r = await postJson(srv.http, "/api/translate", body);
        assert.equal(r.status, status, JSON.stringify(body));
      }
      assert.equal(srv.fetchCalls.length, 0);
    }));

  test("DEF-S07: malformed JSON returns JSON 400 without a stack trace or paths", () =>
    withServer({}, async (srv) => {
      const r = await postJson(srv.http, "/api/translate", "{bad");
      assert.equal(r.status, 400);
      const text = await r.text();
      assert.deepEqual(JSON.parse(text), { error: "invalid_json" });
      assert.ok(!/node_modules|at JSON\.parse/.test(text));
    }));

  test("Phase 8.5: cached text expires after the TTL; size 0 disables the cache", async () => {
    await withServer({ fetch: myMemoryOk("hola"), translationCacheTtlMs: 60 }, async (srv) => {
      await postJson(srv.http, "/api/translate", { text: "hello", from: "en", to: "es" });
      await postJson(srv.http, "/api/translate", { text: "hello", from: "en", to: "es" });
      assert.equal(srv.fetchCalls.length, 1);
      await new Promise((r) => setTimeout(r, 90));
      await postJson(srv.http, "/api/translate", { text: "hello", from: "en", to: "es" });
      assert.equal(srv.fetchCalls.length, 2, "expired entry was not reused");
    });
    await withServer({ fetch: myMemoryOk("hola"), translationCacheMax: 0 }, async (srv) => {
      await postJson(srv.http, "/api/translate", { text: "hello", from: "en", to: "es" });
      await postJson(srv.http, "/api/translate", { text: "hello", from: "en", to: "es" });
      assert.equal(srv.fetchCalls.length, 2);
    });
  });

  test("Phase 8.5: provider error bodies are never logged (they could echo the user's text)", () =>
    withServer({ fetch: async () => jsonResponse({ responseStatus: 403, responseDetails: "INVALID QUERY: secret phrase here" }) }, async (srv) => {
      const r = await postJson(srv.http, "/api/translate", { text: "secret phrase here", from: "en", to: "es" });
      assert.equal(r.status, 502);
      assert.ok(!srv.logs.join("\n").includes("secret phrase"), srv.logs.join("\n"));
      assert.ok(srv.logs.some((l) => l.includes("translate failed via mymemory")));
    }));

  test("DEF-C02 (server half): provider failure is an explicit error, not source text dressed as success", () =>
    withServer({ fetch: async () => jsonResponse({ responseStatus: 429, responseDetails: "QUOTA" }) }, async (srv) => {
      const r = await postJson(srv.http, "/api/translate", { text: "hello", from: "en", to: "es" });
      assert.equal(r.status, 502);
      const body = await r.json();
      assert.deepEqual(body, { error: "translation_failed" });
      // Failure is not cached: a recovered provider is used on the next call.
      const r2 = await postJson(srv.http, "/api/translate", { text: "hello", from: "en", to: "es" });
      assert.equal(r2.status, 502);
      assert.equal(srv.fetchCalls.length, 2);
    }));

  test("malformed provider payloads are treated as failures", () =>
    withServer({ fetch: async () => jsonResponse({ responseStatus: 200, responseData: null }) }, async (srv) => {
      const r = await postJson(srv.http, "/api/translate", { text: "hello", from: "en", to: "es" });
      assert.equal(r.status, 502);
    }));

  test("Gemini failure falls back to MyMemory and is flagged degraded; upstream error text is not exposed", () => {
    const fetchImpl = async (url) => {
      if (String(url).includes("generativelanguage")) return new Response("internal project details", { status: 500 });
      return jsonResponse({ responseStatus: 200, responseData: { translatedText: "hola" } });
    };
    return withServer({ geminiApiKey: "k", fetch: fetchImpl }, async (srv) => {
      const r = await postJson(srv.http, "/api/translate", { text: "hello", from: "en", to: "es" });
      const body = await r.json();
      assert.equal(r.status, 200);
      assert.deepEqual(body, { text: "hola", provider: "mymemory", degraded: true });
      assert.equal(srv.fetchCalls[0].init.headers["x-goog-api-key"], "k");
    });
  });

  test("Gemini success parses candidates", () =>
    withServer(
      {
        geminiApiKey: "k",
        fetch: async () => jsonResponse({ candidates: [{ content: { parts: [{ text: " bonjour " }] } }] }),
      },
      async (srv) => {
        const r = await postJson(srv.http, "/api/translate", { text: "hello", from: "en", to: "fr" });
        assert.equal((await r.json()).text, "bonjour");
        const sent = JSON.parse(srv.fetchCalls[0].init.body);
        assert.match(sent.contents[0].parts[0].text, /English text below into French/);
      }
    ));

  test("DeepL free key uses the free host", () =>
    withServer(
      { deeplApiKey: "abc:fx", fetch: async () => jsonResponse({ translations: [{ text: "Hallo" }] }) },
      async (srv) => {
        const r = await postJson(srv.http, "/api/translate", { text: "hello", from: "en", to: "de" });
        assert.equal((await r.json()).text, "Hallo");
        assert.match(srv.fetchCalls[0].url, /api-free\.deepl\.com/);
      }
    ));

  test("DEF-S09: a stalled provider times out instead of hanging the request", () => {
    const hang = (_url, init) =>
      new Promise((_resolve, reject) => init.signal.addEventListener("abort", () => reject(init.signal.reason)));
    return withServer({ translateTimeoutMs: 100, fetch: hang }, async (srv) => {
      const t0 = Date.now();
      const r = await postJson(srv.http, "/api/translate", { text: "hello", from: "en", to: "es" });
      assert.equal(r.status, 504);
      assert.deepEqual(await r.json(), { error: "translation_timeout" });
      assert.ok(Date.now() - t0 < 2000);
    });
  });
});

describe("POST /api/tts", () => {
  test("no key: 503 with browser fallback hint", () =>
    withServer({}, async (srv) => {
      const r = await postJson(srv.http, "/api/tts", { text: "hi", lang: "en" });
      assert.equal(r.status, 503);
      assert.equal((await r.json()).fallback, "browser");
    }));

  test("streams PCM with format headers; key sent only upstream", () => {
    const pcm = new Uint8Array([1, 0, 2, 0, 3, 0]);
    return withServer(
      { elevenLabsApiKey: "el-secret", fetch: async () => new Response(pcm, { status: 200 }) },
      async (srv) => {
        const r = await postJson(srv.http, "/api/tts", { text: "hola", lang: "es-ES" });
        assert.equal(r.status, 200);
        assert.equal(r.headers.get("x-audio-format"), "pcm");
        assert.equal(r.headers.get("x-sample-rate"), "24000");
        assert.deepEqual(new Uint8Array(await r.arrayBuffer()), pcm);
        const call = srv.fetchCalls[0];
        assert.equal(call.init.headers["xi-api-key"], "el-secret");
        assert.equal(JSON.parse(call.init.body).language_code, "es");
        assert.ok(![...r.headers.values()].some((v) => v.includes("el-secret")));
      }
    );
  });

  test("PCM rejected (4xx) retries as MP3", () => {
    let n = 0;
    const fetchImpl = async () =>
      ++n === 1 ? new Response("tier", { status: 403 }) : new Response(new Uint8Array([9, 9]), { status: 200 });
    return withServer({ elevenLabsApiKey: "k", fetch: fetchImpl }, async (srv) => {
      const r = await postJson(srv.http, "/api/tts", { text: "hola", lang: "es" });
      assert.equal(r.headers.get("x-audio-format"), "mp3");
      assert.match(srv.fetchCalls[1].url, /mp3_44100_128/);
    });
  });

  test("upstream 5xx -> 502 with browser fallback", () =>
    withServer({ elevenLabsApiKey: "k", fetch: async () => new Response("x", { status: 500 }) }, async (srv) => {
      const r = await postJson(srv.http, "/api/tts", { text: "hola" });
      assert.equal(r.status, 502);
      assert.equal((await r.json()).fallback, "browser");
    }));

  test("DEF-S08: oversized TTS text is refused before any paid upstream call", () =>
    withServer({ elevenLabsApiKey: "k", maxTextChars: 100 }, async (srv) => {
      const r = await postJson(srv.http, "/api/tts", { text: "a".repeat(101) });
      assert.equal(r.status, 413);
      const r2 = await postJson(srv.http, "/api/tts", { text: 42 });
      assert.equal(r2.status, 400);
      assert.equal(srv.fetchCalls.length, 0);
    }));

  test("DEF-S09: stalled TTS upstream times out", () => {
    const hang = (_url, init) =>
      new Promise((_resolve, reject) => init.signal.addEventListener("abort", () => reject(new Error("aborted"))));
    return withServer({ elevenLabsApiKey: "k", ttsTimeoutMs: 100, fetch: hang }, async (srv) => {
      const r = await postJson(srv.http, "/api/tts", { text: "hola" });
      assert.equal(r.status, 504);
    });
  });
});

describe("POST /api/stt", () => {
  const form = (bytes = 3000, lang = "es") => {
    const fd = new FormData();
    fd.append("file", new Blob([new Uint8Array(bytes)], { type: "audio/webm" }), "push.webm");
    if (lang) fd.append("language_code", lang);
    return fd;
  };

  test("missing file -> 400; no key -> 503", () =>
    withServer({}, async (srv) => {
      const r = await fetch(srv.http + "/api/stt", { method: "POST", body: new FormData() });
      assert.equal(r.status, 400);
      const r2 = await fetch(srv.http + "/api/stt", { method: "POST", body: form() });
      assert.equal(r2.status, 503);
    }));

  test("proxies to Scribe (mocked) and returns only the transcript", () =>
    withServer(
      {
        elevenLabsApiKey: "k",
        fetch: async () => jsonResponse({ text: "hola mundo", language_code: "spa", words: [{ text: "x" }] }),
      },
      async (srv) => {
        const r = await fetch(srv.http + "/api/stt", { method: "POST", body: form() });
        assert.deepEqual(await r.json(), { text: "hola mundo", language_code: "spa" });
        const sent = srv.fetchCalls[0].init.body;
        assert.equal(sent.get("language_code"), "es");
        assert.equal(sent.get("model_id"), "scribe_v1");
      }
    ));

  test("oversized upload -> 413 JSON", () =>
    withServer({ elevenLabsApiKey: "k", maxUploadBytes: 1000 }, async (srv) => {
      const r = await fetch(srv.http + "/api/stt", { method: "POST", body: form(5000) });
      assert.equal(r.status, 413);
      assert.deepEqual(await r.json(), { error: "file_too_large" });
      assert.equal(srv.fetchCalls.length, 0);
    }));

  test("upstream failure -> 502", () =>
    withServer({ elevenLabsApiKey: "k", fetch: async () => new Response("no", { status: 401 }) }, async (srv) => {
      const r = await fetch(srv.http + "/api/stt", { method: "POST", body: form() });
      assert.equal(r.status, 502);
    }));
});

describe("abuse controls", () => {
  test("DEF-S06: foreign origin refused on /api when ALLOWED_ORIGINS is set", () =>
    withServer({ allowedOrigins: ["https://app.example"], fetch: myMemoryOk("hola") }, async (srv) => {
      const bad = await postJson(srv.http, "/api/translate", { text: "hi", from: "en", to: "es" }, { Origin: "https://evil.example" });
      assert.equal(bad.status, 403);
      const good = await postJson(srv.http, "/api/translate", { text: "hi", from: "en", to: "es" }, { Origin: "https://app.example" });
      assert.equal(good.status, 200);
      assert.equal(good.headers.get("access-control-allow-origin"), "https://app.example");
      assert.equal(srv.fetchCalls.length, 1);
    }));

  test("default (no allowlist) keeps the previous permissive CORS behavior", () =>
    withServer({}, async (srv) => {
      const r = await fetch(srv.http + "/api/capabilities", { headers: { Origin: "https://x.example" } });
      assert.equal(r.headers.get("access-control-allow-origin"), "*");
    }));

  test("rate limiter returns 429 past the per-minute budget when enabled", () =>
    withServer({ rateLimitPerMinute: 3 }, async (srv) => {
      const statuses = [];
      for (let i = 0; i < 5; i++) {
        statuses.push((await postJson(srv.http, "/api/translate", { text: "hi", from: "en", to: "en" })).status);
      }
      assert.deepEqual(statuses, [200, 200, 200, 429, 429]);
      // capabilities is exempt
      assert.equal((await fetch(srv.http + "/api/capabilities")).status, 200);
    }));
});

describe("VoiceMirror TTS hints (Phase 3)", () => {
  const pcm = () => new Response(new Uint8Array([1, 0]), { status: 200 });
  const CATALOG = { low: "lowVoice00001", mid: "midVoice00001", high: "highVoice0001" };

  test("disabled (default): a voice hint is ignored — fixed voice, speed 1.0", () =>
    withServer({ elevenLabsApiKey: "k", fetch: async () => pcm() }, async (srv) => {
      const r = await postJson(srv.http, "/api/tts", { text: "hola", lang: "es", voice: { register: "high", speed: 1.15 } });
      assert.equal(r.status, 200);
      const call = srv.fetchCalls[0];
      assert.match(call.url, /21m00Tcm4TlvDq8ikWAM/);
      assert.equal(JSON.parse(call.init.body).voice_settings.speed, 1.0);
    }));

  test("enabled: register picks the SERVER catalog voice and speed is applied", () =>
    withServer({ elevenLabsApiKey: "k", voiceMirrorEnabled: true, voiceCatalog: CATALOG, fetch: async () => pcm() }, async (srv) => {
      const caps = await (await fetch(srv.http + "/api/capabilities")).json();
      assert.equal(caps.voiceHints, true);
      await postJson(srv.http, "/api/tts", { text: "hola", lang: "es", voice: { register: "low", speed: 0.9 } });
      const call = srv.fetchCalls[0];
      assert.match(call.url, /lowVoice00001/);
      assert.equal(JSON.parse(call.init.body).voice_settings.speed, 0.9);
    }));

  test("enabled without a catalog: default voice, speed still applied", () =>
    withServer({ elevenLabsApiKey: "k", voiceMirrorEnabled: true, voiceCatalog: null, fetch: async () => pcm() }, async (srv) => {
      await postJson(srv.http, "/api/tts", { text: "hola", voice: { register: "high", speed: 1.2 } });
      assert.match(srv.fetchCalls[0].url, /21m00Tcm4TlvDq8ikWAM/);
      assert.equal(JSON.parse(srv.fetchCalls[0].init.body).voice_settings.speed, 1.2);
    }));

  test("malformed hints are refused before any paid call; clients cannot choose a voice id", () =>
    withServer({ elevenLabsApiKey: "k", voiceMirrorEnabled: true, voiceCatalog: CATALOG, fetch: async () => pcm() }, async (srv) => {
      const bad = [
        { register: "loud", speed: 1 },
        { register: "low", speed: 3 },
        { register: "low", speed: "1" },
        { register: "low", speed: 1, voiceId: "attackerVoice1" },
        ["low", 1],
        "high",
      ];
      for (const voice of bad) {
        const r = await postJson(srv.http, "/api/tts", { text: "hola", voice });
        assert.equal(r.status, 400, JSON.stringify(voice));
      }
      assert.equal(srv.fetchCalls.length, 0);
    }));
});
