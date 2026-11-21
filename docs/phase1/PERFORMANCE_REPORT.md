# Performance Report — Talktual Phase 1

## 1. Setup

* **Harness:** `e2e/perf.mjs` — spawns a server process with no provider keys and drives it from a separate Node client process over loopback. Same script run against the **baseline** server (`git show 34a8400:signaling-server/server.js`, using the current `node_modules`) and the **repaired** server.
* **Machine:** 4-vCPU Linux container, Node 22.23.5. Shared cloud hardware: treat absolute numbers as indicative; the A/B comparison is the useful part.
* **Not measured:** provider latency (no live calls), real-browser audio latency, mobile devices, real networks. Translation latency below uses same-language passthrough so it measures server overhead only.

## 2. Results (single run each)

| Metric | Baseline | Repaired | Note |
| --- | --- | --- | --- |
| Startup → first `/health` 200 | 120.6 ms | 150.7 ms | Within noise of process spawn; no new startup work besides config parsing |
| Idle RSS | 57.3 MB | 57.2 MB | |
| `GET /health` p50 / p95 / p99 | 0.61 / 0.99 / 3.09 ms | 0.67 / 1.05 / 2.06 ms | n = 2000 |
| `GET /api/capabilities` p50 / p95 / p99 | 0.27 / 0.69 / 1.98 ms | 0.30 / 0.76 / 1.98 ms | n = 2000 |
| `POST /api/translate` (passthrough) p50 / p95 / p99 | 0.52 / 0.98 / 2.46 ms | 0.60 / 1.00 / 2.92 ms | +~0.08 ms p50 from validation + headers middleware |
| WS relay round trip (A→server→B→server→A) p50 / p95 / p99 | 0.11 / 0.27 / 0.65 ms | 0.11 / 0.17 / 0.31 ms | n = 2900; validation adds no measurable cost |
| 1000 sequential join/leave cycles | 1462 ms, RSS 109.7 → 109.9 MB | 1466 ms, RSS 111.8 → 112.8 MB | No growth trend at RSS resolution; `sessions` map empty afterwards (asserted in tests/E2E) |
| 250 concurrent connected pairs | RSS 109.9 → 110.0 MB | RSS 112.8 → 112.7 MB | Per-pair cost is below RSS resolution (V8 heap already reserved); server alive throughout |

Client bundle: baseline 570.89 kB JS (195.45 kB gzip) → 575.05 kB (196.88 kB gzip). +4 kB from validation, room-code and error-handling code; the unused dependencies removed were never bundled (tree-shaken).

## 3. Bottlenecks and reliability findings

Measured server overhead is sub-millisecond; **the server is not a latency bottleneck at these loads.** The latency-relevant findings were reliability defects, now fixed:

| Finding | Effect | Status |
| --- | --- | --- |
| No provider timeouts + serialized outgoing segments (DEF-S09) | One stalled translation blocked every later turn in the call indefinitely | Fixed: 8 s server / 10 s client budget |
| Barge-in didn't cancel queued TTS (DEF-C01) | Stale audio kept playing (and downloading) after the user cut in | Fixed; in-flight TTS request aborted, server aborts upstream |
| GainNode per segment never released (DEF-C10) | Audio-graph growth over a long call | Fixed |
| Unbounded WS frames (DEF-S02) | Memory/bandwidth spike from one client | Fixed (64 KiB) |

Bounded structures verified: server translation cache (2000, FIFO), client cache (500, FIFO), rate-limit buckets purged every 30 s, sessions removed on empty/expiry, dead sockets reaped by 30 s ping. Unbounded: transcript `messages` array per call (text only; OBS-06).

## 4. Caveats

* RSS is a coarse instrument; it cannot show small per-session leaks. The join/leave test's stronger evidence is the functional one: the `sessions` map returns to 0 (server tests + E2E step 11).
* Single runs; no statistical confidence intervals. Re-run `node e2e/perf.mjs` to compare on target hardware.
* Long-session behaviour in a real browser (hours of continuous Web Speech, audio contexts on mobile) was **not** measured.

## 5. Proposed priorities (not done in Phase 1)

1. Measure real end-to-end turn latency (release → first audio) on target devices with real providers — this is the product's key metric and is currently unmeasured.
2. Code-split the 575 kB bundle: the QR scanner (`@yudiel/react-qr-scanner`, only on the Join screen) and `motion` are the likely large contributors — lazy-load the scanner first.
3. Cap the transcript length in long calls (OBS-06).
4. Consider a TTL on the server translation cache (also a privacy win).
