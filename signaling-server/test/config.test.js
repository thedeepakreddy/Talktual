const { test, describe } = require("node:test");
const assert = require("node:assert/strict");
const { spawnSync } = require("node:child_process");
const path = require("node:path");
const { validateEnv } = require("../server");

describe("validateEnv (Phase 1 condition: production settings)", () => {
  test("empty dev environment: no errors, production gaps are warnings", () => {
    const r = validateEnv({});
    assert.deepEqual(r.errors, []);
    assert.ok(r.warnings.some((w) => w.includes("ALLOWED_ORIGINS")));
    assert.ok(r.warnings.some((w) => w.includes("RATE_LIMIT_PER_MINUTE")));
  });

  test("production mode: missing allowlist and rate limit are errors", () => {
    const r = validateEnv({}, { production: true });
    assert.equal(r.errors.length, 2);
  });

  test("a complete production configuration passes", () => {
    const r = validateEnv(
      { ALLOWED_ORIGINS: "https://app.example.com,https://www.example.com", RATE_LIMIT_PER_MINUTE: "600", TRUST_PROXY: "1" },
      { production: true }
    );
    assert.deepEqual(r.errors, []);
    assert.deepEqual(r.warnings, []);
  });

  test("typos are errors, not silently defaulted", () => {
    const cases = [
      { RATE_LIMIT_PER_MINUTE: "six hundred" },
      { RATE_LIMIT_PER_MINUTE: "-5" },
      { MAX_TEXT_CHARS: "0" },
      { ALLOWED_ORIGINS: "https://app.example.com/path" },
      { ALLOWED_ORIGINS: "*.example.com" },
      { ALLOWED_ORIGINS: "app.example.com" },
      { TRUST_PROXY: "yes please" },
      { PORT: "eighty" },
      { RESUME_GRACE_MS: "soon" },
    ];
    for (const env of cases) assert.equal(validateEnv(env).errors.length, 1, JSON.stringify(env));
  });

  test("accepts valid TRUST_PROXY forms", () => {
    for (const TRUST_PROXY of ["true", "false", "1", "2", "loopback", "10.0.0.0/8", "10.0.0.1, 192.168.0.0/16"]) {
      assert.deepEqual(validateEnv({ TRUST_PROXY }).errors, [], TRUST_PROXY);
    }
  });

  test("warns on plain-http non-localhost origins, rate limit without TRUST_PROXY, VITE_ key, resume disabled", () => {
    const r = validateEnv({
      ALLOWED_ORIGINS: "http://app.example.com",
      RATE_LIMIT_PER_MINUTE: "600",
      VITE_ELEVENLABS_API_KEY: "x",
      RESUME_GRACE_MS: "0",
    });
    assert.deepEqual(r.errors, []);
    assert.equal(r.warnings.length, 4);
  });

  test("never echoes secret values", () => {
    const r = validateEnv({ ELEVENLABS_API_KEY: "el-super-secret", GEMINI_API_KEY: "gm-secret" });
    const out = JSON.stringify(r);
    assert.ok(!out.includes("el-super-secret"));
    assert.ok(!out.includes("gm-secret"));
    assert.equal(r.secretsPresent.ELEVENLABS_API_KEY, true);
  });
});

describe("check-config CLI and startup gate", () => {
  const cwd = path.join(__dirname, "..");
  const run = (args, env) =>
    spawnSync(process.execPath, args, { cwd: path.join(cwd, "test"), env: { PATH: process.env.PATH, ...env }, encoding: "utf8", timeout: 10000 });

  test("check-config --production fails with exit 1 when settings are missing, passes when complete", () => {
    const bad = run([path.join(cwd, "scripts/check-config.js"), "--production"], {});
    assert.equal(bad.status, 1);
    assert.match(bad.stdout, /ERROR\s+ALLOWED_ORIGINS/);
    const good = run([path.join(cwd, "scripts/check-config.js"), "--production"], {
      ALLOWED_ORIGINS: "https://app.example.com",
      RATE_LIMIT_PER_MINUTE: "600",
      TRUST_PROXY: "1",
      ELEVENLABS_API_KEY: "el-secret-value",
    });
    assert.equal(good.status, 0, good.stdout);
    assert.ok(!good.stdout.includes("el-secret-value"));
    assert.match(good.stdout, /ELEVENLABS_API_KEY=yes/);
  });

  test("the server refuses to start on an invalid value", () => {
    const r = run([path.join(cwd, "server.js")], { PORT: "0", ALLOWED_ORIGINS: "not-an-origin" });
    assert.equal(r.status, 1);
    assert.match(r.stdout, /Refusing to start/);
  });
});

test("startup with NODE_ENV=production but no allowlist still starts (warnings only)", async () => {
  const { spawn } = require("node:child_process");
  const child = spawn(process.execPath, [path.join(__dirname, "..", "server.js")], {
    cwd: __dirname,
    env: { PATH: process.env.PATH, PORT: "0", NODE_ENV: "production" },
  });
  let out = "";
  child.stdout.on("data", (d) => (out += d));
  await new Promise((r) => setTimeout(r, 1500));
  child.kill();
  assert.match(out, /signaling \+ API proxy on/);
  assert.match(out, /WARNING: ALLOWED_ORIGINS is not set/);
});
