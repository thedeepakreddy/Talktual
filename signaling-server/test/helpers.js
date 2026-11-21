const WebSocket = require("ws");
const { createServer } = require("../server");

/** Starts an isolated server on an ephemeral port. Never touches real providers. */
async function start(overrides = {}) {
  const logs = [];
  const fetchCalls = [];
  const fetchImpl =
    overrides.fetch ||
    (async (url) => {
      throw new Error(`unexpected upstream call in test: ${url}`);
    });
  const instance = createServer({
    // Never inherit real keys from the developer's environment.
    elevenLabsApiKey: undefined,
    geminiApiKey: undefined,
    deeplApiKey: undefined,
    allowedOrigins: [],
    rateLimitPerMinute: 0,
    ...overrides,
    fetch: async (url, init) => {
      fetchCalls.push({ url: String(url), init });
      return fetchImpl(url, init);
    },
    log: (...args) => logs.push(args.join(" ")),
  });
  await new Promise((r) => instance.server.listen(0, "127.0.0.1", r));
  const { port } = instance.server.address();
  return {
    ...instance,
    logs,
    fetchCalls,
    http: `http://127.0.0.1:${port}`,
    wsUrl: `ws://127.0.0.1:${port}`,
  };
}

function connect(url, opts = {}) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url, opts);
    ws.msgs = [];
    ws.on("message", (d) => ws.msgs.push(JSON.parse(d.toString())));
    ws.on("open", () => resolve(ws));
    ws.on("error", reject);
  });
}

const wait = (ms) => new Promise((r) => setTimeout(r, ms));

async function waitFor(pred, timeoutMs = 2000) {
  const end = Date.now() + timeoutMs;
  while (Date.now() < end) {
    if (pred()) return true;
    await wait(10);
  }
  throw new Error("waitFor timed out");
}

function postJson(base, path, body, headers = {}) {
  return fetch(base + path, {
    method: "POST",
    headers: { "Content-Type": "application/json", ...headers },
    body: typeof body === "string" ? body : JSON.stringify(body),
  });
}

function jsonResponse(obj, status = 200) {
  return new Response(JSON.stringify(obj), { status, headers: { "Content-Type": "application/json" } });
}

module.exports = { start, connect, wait, waitFor, postJson, jsonResponse };
