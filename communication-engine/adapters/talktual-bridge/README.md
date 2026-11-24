# talktual-bridge — Talktual ↔ Communication Engine, end to end

A local WebSocket bridge that lets **Talktual use the Communication Engine
(CE) for speech recognition**. ADR: [0024](../../docs/architecture/adr/0024-talktual-ce-bridge.md).

```
Talktual (browser)                      talktual-bridge (this computer)
 mic → AudioWorklet → PCM16 ──WebSocket──▶ PushBackend (CE audio backend)
                                           → CE capture stream (consent gate, negotiation)
                                           → Phase 3 analysis (hands-free mode)
                                           → Phase 4 recognition (sherpa-onnx Zipformer, English)
 AsrCallbacks ◀── partial / final JSON ◀── speech events
   → translation → peer → TTS (unchanged Talktual pipeline)
```

* Opt-in: Talktual uses CE only when built with `VITE_ASR_ENGINE=ce`.
  Otherwise nothing changes; the browser recognizer stays the default.
* Push-to-talk streams only while the button is held (plus Talktual's
  450 ms release tail). Hands-free mode uses CE's Phase 3 activity
  detection to split phrases.
* The bridge listens on `127.0.0.1:8091` and accepts browser connections
  only from Talktual's local origins (5173, 4173, 8080), plus any you add.
* English only (the model's language). Talktual shows a clear error for
  other languages.
* Nothing is stored, unless you start the bridge with `--log-finals` for a
  human test (see below).
* Outside the CE workspace, like `ce-asr-sherpa`: it needs Rust ≥ 1.89 and
  the native sherpa-onnx libraries.

## Human test — step by step

Verified here on Linux x86_64 with a synthetic microphone only. Your run
is the first test with a real person and a real microphone.

**1. Prerequisites:** Rust (<https://rustup.rs>), Node 20+, about 400 MB of
free disk space, and a microphone. Linux x86_64 is verified. macOS and
Windows should work (sherpa-onnx ships native libraries for them) but have
not been tested.

**2. Get the model** (approved INT8 files only, checksum-verified):

```bash
communication-engine/adapters/ce-asr-sherpa/scripts/fetch-model.sh ~/ce-models
export CE_SHERPA_MODEL_DIR=~/ce-models/sherpa-onnx-streaming-zipformer-en-2023-06-26
# only if the script printed it (Linux x86_64):
export SHERPA_ONNX_ARCHIVE_DIR=~/ce-models/native
```

**3. Start everything** (bridge, signaling server, Talktual with the CE
recognizer). The first build takes a few minutes:

```bash
communication-engine/adapters/talktual-bridge/scripts/run-talktual-ce.sh
```

**4. Talk.** Open <http://localhost:5173> in two browser windows. Start a
call in one window (language: **English**) and join from the other.
Accept the microphone sheet; it should say your voice goes to "the
Communication Engine bridge running on this computer". Hold **Push to
talk**, speak, release. Your words appear as you speak and are translated
for the other window. Hands-free mode works too.

**4b. Engine Lab: test every phase.** Open
<http://localhost:5173/#/engine> (also reachable from Settings ›
Communication Engine › Engine Lab). Press **Connect**, then hold **Hold to
talk**, or switch to **Hands-free** and press Start. The page shows, live from
the engine:

| Panel | What you can check |
|---|---|
| Phase 5 · Semantic & contextual interpretation | observations from the on-device rule adapter (switch "Interpret meaning"): numbers, quantities, money, times, dates, durations, digit sequences; references such as "it" or "then" resolved against this session's context, or left ambiguous/unresolved; the utterance's act (question, request, …) or an abstention. Each shows its status (provisional → stable → final, or retracted), epistemic kind, certainty, version and source revision. **Confirm** marks an entity as user-confirmed; **Clear context** empties the session context |
| Phase 4 · Speech recognition & language | the live transcript (committed text in bold, provisional tail muted); finals with reason, revision count, audio length and utterance language; latency percentiles, mode, drops; stream language; recognizer capabilities. **Reset recognizer** and **Abort** exercise cancellation |
| Phase 3 · Audio intelligence | voice activity, level, SNR, noise floor, usability and reasons, active conditions, closed segments (toggle with "Run audio intelligence") |
| Phase 2 · Audio capture | stream state, negotiated device/delivered format, frames, blocks, overflow, discontinuities, ring fill |
| Phase 1 · Runtime, sessions, health | runtime state, health, sessions, engine metrics (**Refresh health**) |
| CE event log | every engine event of your session, newest first, by topic, with its privacy class (transcript text is never shown in the log) |

Things worth trying:
* "set a timer for twenty five minutes", then "no I said fifteen minutes"
  (a duration that updates while you speak, then a correction);
* "the total is forty dollars", then "can you split it" (a reference
  resolved to the earlier amount); then "and twelve euros", "pay it" (now
  ambiguous);
* talking in a noisy room (quality, conditions);
* pausing mid-sentence (segments, endpoints, revisions);
* very short words like "yes" or "no" (language abstention);
* Reset while talking.

**5. Measure (optional).** To score the test, run the bridge yourself with
the transcript log, read the sentences in
[`human-test/sentences-en.txt`](human-test/sentences-en.txt) in order (one
push per sentence), then score:

```bash
cd communication-engine/adapters/talktual-bridge
cargo run --release -- --log-finals /tmp/finals.jsonl   # instead of the bridge started by the script
# … run the signaling server and `VITE_ASR_ENGINE=ce npm run dev` as in run-talktual-ce.sh …
cargo run --release --example score_human_test -- human-test/sentences-en.txt /tmp/finals.jsonl
```

The scorer prints WER/CER, using the same normalization as the Phase 4
evaluation, and wall-clock latencies:
* first partial after the utterance opened (with push-to-talk, the clock
  starts at the press, so it includes your reaction time);
* final after release.

These are the same 10 sentences as the synthetic fixtures, so your numbers
compare directly with the TTS results in
`docs/phases/phase-4-real-asr.md` (held-out clean WER 0.121). The log
contains your transcripts: delete it when you are done.

## Run the pieces yourself

```bash
cargo run --release -- [--listen 127.0.0.1:8091] [--allow-origin http://my-host:5173] [--log-finals FILE]
VITE_ASR_ENGINE=ce VITE_CE_BRIDGE_URL=ws://127.0.0.1:8091 npm run dev     # in the repository root
```

To use a phone, Talktual needs HTTPS (secure context) and the bridge
must be reachable from it (`--listen 0.0.0.0:8091 --allow-origin
https://…`). That setup has not been tested; prefer one computer for the
first test.

## Protocol (v1)

See [`src/protocol.rs`](src/protocol.rs):
* client: `start` (lang, sampleRate, segmentation), then binary PCM16 LE
  mono frames, then `stop` or `abort`;
* bridge: `ready`, `utterance`, `partial` (committed + provisional),
  `final` (text, reason, language, durationMs), `cancelled`, `activity`,
  `warning`, `error`, `end`;
* optional diagnostics (Engine Lab): `info`, `health` and `reset`
  requests; `"diagnostics": true` adds `ce-event` (every CE event of the
  session, flattened as topic, kind, privacy and data) and a 1 s `status`
  (audio stream, analysis, recognition, metrics); `"analysis": true` runs
  Phase 3 in push-to-talk too. Clients that don't ask get exactly the
  v1 messages;
* optional semantics (Phase 5): `"semantics": true` in `start` runs CE
  semantic interpretation, whose events arrive as `ce-event`s with topic
  `semantic`; `status` gains `semantic`; `reset-context` and
  `confirm` (`observation`) requests; `end` follows the drained
  interpretation.

## Verify

```bash
scripts/verify.sh            # fmt, clippy, protocol tests (scripted adapter), real-model tests, cargo deny
node e2e/ce-bridge.mjs       # repository root: Chromium + Talktual + bridge + real model, synthetic mic
```

## Known limits

* The model's accuracy on human speech has not been measured yet; your
  test is that measurement. On Chromium's fake microphone (a TTS fixture
  through the browser's noise suppression), it heard "train station" as
  "drain station" in some runs.
* The model outputs upper-case words without punctuation. Talktual shows
  them in sentence case; no punctuation is added.
* One browser tab per push opens one WebSocket and one CE session. The
  model is loaded once and shared.
