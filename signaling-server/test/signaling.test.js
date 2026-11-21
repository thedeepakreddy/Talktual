const { test, describe, before, after } = require("node:test");
const assert = require("node:assert/strict");
const net = require("node:net");
const { start, connect, wait, waitFor } = require("./helpers");

describe("websocket signaling", () => {
  let srv;
  before(async () => {
    srv = await start();
  });
  after(() => srv.close());

  const join = (ws, sessionId) => ws.send(JSON.stringify({ type: "join", sessionId }));

  test("two peers join: roles assigned, both get ready, messages relayed as text", async () => {
    const a = await connect(srv.wsUrl);
    const b = await connect(srv.wsUrl);
    join(a, "ABC123");
    await waitFor(() => a.msgs.length === 1);
    join(b, "ABC123");
    await waitFor(() => a.msgs.length === 2 && b.msgs.length === 2);
    assert.deepEqual(a.msgs, [{ type: "role", role: "offerer" }, { type: "ready" }]);
    assert.deepEqual(b.msgs, [{ type: "role", role: "answerer" }, { type: "ready" }]);

    a.send(JSON.stringify({ type: "offer", offer: { type: "offer", sdp: "v=0" } }));
    a.send(JSON.stringify({ type: "app", payload: { t: "partial", text: "hi", lang: "en-US" } }));
    await waitFor(() => b.msgs.length === 4);
    assert.equal(b.msgs[2].type, "offer");
    assert.equal(b.msgs[3].payload.text, "hi");
    assert.equal(a.msgs.length, 2, "sender must not receive its own relay");
    a.close();
    b.close();
  });

  test("third joiner is refused with session_full", async () => {
    const [a, b, c] = await Promise.all([connect(srv.wsUrl), connect(srv.wsUrl), connect(srv.wsUrl)]);
    join(a, "FULL01");
    join(b, "FULL01");
    await waitFor(() => b.msgs.some((m) => m.type === "ready"));
    join(c, "FULL01");
    await waitFor(() => c.msgs.length === 1);
    assert.equal(c.msgs[0].type, "error");
    assert.equal(c.msgs[0].code, "session_full");
    [a, b, c].forEach((w) => w.close());
  });

  test("DEF-S03: join without / with malformed sessionId is rejected (strangers are not paired)", async () => {
    const a = await connect(srv.wsUrl);
    const b = await connect(srv.wsUrl);
    a.send(JSON.stringify({ type: "join" }));
    b.send(JSON.stringify({ type: "join" }));
    a.send(JSON.stringify({ type: "join", sessionId: { $gt: "" } }));
    a.send(JSON.stringify({ type: "join", sessionId: "x".repeat(500) }));
    await waitFor(() => a.msgs.length === 3 && b.msgs.length === 1);
    assert.ok(a.msgs.every((m) => m.type === "error" && m.code === "invalid_session"));
    assert.ok(!b.msgs.some((m) => m.type === "ready"));
    a.close();
    b.close();
  });

  test("DEF-S04: a socket cannot join twice (no self-pairing, no leaked seat)", async () => {
    const a = await connect(srv.wsUrl);
    join(a, "DUP001");
    join(a, "DUP001");
    join(a, "DUP002");
    await waitFor(() => a.msgs.length === 3);
    assert.deepEqual(a.msgs[0], { type: "role", role: "offerer" });
    assert.equal(a.msgs[1].code, "already_joined");
    assert.equal(a.msgs[2].code, "already_joined");
    assert.ok(!a.msgs.some((m) => m.type === "ready"));
    assert.equal(srv.sessions.has("DUP002"), false);
    a.close();
    await waitFor(() => !srv.sessions.has("DUP001"));
  });

  test("DEF-S04: a fresh joiner is never paired with a dead socket", async () => {
    const a = await connect(srv.wsUrl);
    join(a, "GHOST1");
    await waitFor(() => a.msgs.length === 1);
    a.close();
    await waitFor(() => !srv.sessions.has("GHOST1"));
    const b = await connect(srv.wsUrl);
    join(b, "GHOST1");
    await waitFor(() => b.msgs.length === 1);
    await wait(50);
    assert.deepEqual(b.msgs, [{ type: "role", role: "offerer" }]);
    b.close();
  });

  test("DEF-S05: only signaling types are relayed", async () => {
    const a = await connect(srv.wsUrl);
    const b = await connect(srv.wsUrl);
    join(a, "TYPES1");
    join(b, "TYPES1");
    await waitFor(() => b.msgs.some((m) => m.type === "ready"));
    const before = b.msgs.length;
    a.send(JSON.stringify({ type: "peer-left" }));
    a.send(JSON.stringify({ type: "expired" }));
    a.send(JSON.stringify({ type: "whatever", x: 1 }));
    a.send("not json");
    a.send(JSON.stringify(null));
    a.send(JSON.stringify({ type: "ice", candidate: { candidate: "c" } }));
    await waitFor(() => b.msgs.length === before + 1);
    await wait(50);
    assert.equal(b.msgs.length, before + 1);
    assert.equal(b.msgs.at(-1).type, "ice");
    a.close();
    b.close();
  });

  test("peer-left is sent to the remaining peer; session removed when empty", async () => {
    const a = await connect(srv.wsUrl);
    const b = await connect(srv.wsUrl);
    join(a, "LEAVE1");
    join(b, "LEAVE1");
    await waitFor(() => b.msgs.some((m) => m.type === "ready"));
    a.close();
    await waitFor(() => b.msgs.some((m) => m.type === "peer-left"));
    assert.equal(srv.sessions.get("LEAVE1").peers.length, 1);
    b.close();
    await waitFor(() => !srv.sessions.has("LEAVE1"));
  });

  test("DEF-S02: oversized frames are refused and the server survives", async () => {
    const a = await connect(srv.wsUrl);
    const b = await connect(srv.wsUrl);
    join(a, "BIG001");
    join(b, "BIG001");
    await waitFor(() => b.msgs.some((m) => m.type === "ready"));
    const closed = new Promise((r) => a.on("close", (code) => r(code)));
    a.on("error", () => {});
    a.send(JSON.stringify({ type: "app", payload: { t: "partial", text: "x".repeat(200 * 1024) } }));
    assert.equal(await closed, 1009);
    await wait(50);
    assert.ok(!b.msgs.some((m) => m.type === "app"), "oversized payload must not be relayed");
    const res = await fetch(srv.http + "/health");
    assert.equal(res.status, 200);
    b.close();
  });

  test("DEF-S01: a malformed frame (invalid UTF-8) does not crash the process", async () => {
    const a = await connect(srv.wsUrl);
    a.on("error", () => {});
    const closed = new Promise((r) => a.on("close", (code) => r(code)));
    const mask = Buffer.from([1, 2, 3, 4]);
    const payload = Buffer.from([0xc3, 0x28]).map((byte, i) => byte ^ mask[i % 4]);
    a._socket.write(Buffer.concat([Buffer.from([0x81, 0x80 | 2]), mask, payload]));
    assert.equal(await closed, 1007);
    await wait(50);
    const res = await fetch(srv.http + "/health");
    assert.equal(res.status, 200, "server must still be serving");
    assert.ok(srv.logs.some((l) => l.includes("WS_ERR_INVALID_UTF8")));
  });

  test("garbage on the raw TCP port doesn't crash the process", async () => {
    await new Promise((resolve) => {
      const sock = net.connect(Number(new URL(srv.http).port), "127.0.0.1", () => {
        sock.write("GET / HTTP/1.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n\x00\xff\xfe");
        setTimeout(() => {
          sock.destroy();
          resolve();
        }, 50);
      });
      sock.on("error", resolve);
    });
    const res = await fetch(srv.http + "/health");
    assert.equal(res.status, 200);
  });
});

describe("session expiry", () => {
  test("a lone peer is told the session expired and the entry is removed", async () => {
    const srv = await start({ sessionTimeoutMs: 50, cleanupMs: 30 });
    try {
      const a = await connect(srv.wsUrl);
      a.send(JSON.stringify({ type: "join", sessionId: "EXP001" }));
      await waitFor(() => a.msgs.some((m) => m.type === "expired"), 2000);
      assert.equal(srv.sessions.has("EXP001"), false);
      // The expired socket can't relay into a recreated session with the same id.
      const b = await connect(srv.wsUrl);
      b.send(JSON.stringify({ type: "join", sessionId: "EXP001" }));
      await waitFor(() => b.msgs.length === 1);
      assert.deepEqual(b.msgs[0], { type: "role", role: "offerer" });
      a.send(JSON.stringify({ type: "app", payload: { t: "partial", text: "stale" } }));
      a.close();
      await wait(80);
      // (b may itself expire under the 50ms test timeout; that's fine.)
      assert.ok(
        !b.msgs.some((m) => m.type === "app" || m.type === "peer-left"),
        `no relay and no spurious peer-left from the expired socket: ${JSON.stringify(b.msgs)}`
      );
      b.close();
    } finally {
      await srv.close();
    }
  });
});

describe("origin allowlist", () => {
  test("DEF-S06: websocket from a foreign origin is refused when ALLOWED_ORIGINS is set", async () => {
    const srv = await start({ allowedOrigins: ["https://app.example"] });
    try {
      await assert.rejects(connect(srv.wsUrl, { origin: "https://evil.example" }));
      const ok = await connect(srv.wsUrl, { origin: "https://app.example" });
      ok.close();
    } finally {
      await srv.close();
    }
  });
});
