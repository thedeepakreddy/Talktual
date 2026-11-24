# ADR-0024 — Talktual uses CE recognition through a local bridge

Status: Accepted (Phase 4) · Date: 2026-11-24 · Requested by the user, to
run a human test of the real-ASR baseline inside Talktual.

## Context
Talktual is a browser app (Vite/React) with a Node signaling server. CE is
native Rust: its runtime does not run in the browser, and the sherpa-onnx
adapter links native libraries (ADR-0023). Talktual already isolates
recognition behind `AsrAdapter` / `createAsr` (Web Speech, ElevenLabs Scribe
fallback) with per-route consent and disclosure.

## Decision
* **Bridge** (`adapters/talktual-bridge`, outside the CE workspace): a
  local WebSocket server hosting one CE `Engine` with the sherpa-onnx
  adapter, keeping the model loaded.
  * A `PushBackend` implements CE's `AudioBackend`: its one input device
    is fed by PCM from the network. Audio therefore enters CE through the
    normal capture path: consent gate, format negotiation, bounded ring,
    timestamps, taps, Phase 3 analysis and Phase 4 recognition.
  * Each WebSocket is one CE session with one capture stream. A graceful
    stop waits until CE has consumed every pushed frame, then closes the
    stream, so the recognizer finalizes everything (`StreamEnd`).
  * For network bursts, the capture ring is 5 s and the recognition queue
    1000 blocks.
  * The bridge binds to 127.0.0.1 and checks browser `Origin`s.
  * Transcripts go back only to the client that sent the audio.
  * An opt-in `--log-finals` writes finals with wall-clock latencies for
    scoring a human test. It is off by default.
* **Talktual**: a new opt-in recognizer, engine `ce-local` with its own
  processing route, consent key and disclosure ("streamed … to the
  Communication Engine bridge running on this computer"), selected only
  when built with `VITE_ASR_ENGINE=ce`.
  * It captures through an AudioWorklet (ScriptProcessor fallback) from
    the call's existing microphone stream and sends 16-bit PCM.
  * Push-to-talk keeps Talktual's 450 ms release tail and uses CE
    `Continuous` delimitation (the press delimits; the model's endpoint
    splits phrases). Hands-free mode uses CE `ActivityHints`.
  * Partials and finals map onto the existing `AsrCallbacks`; translation,
    peer transport and TTS are unchanged. Upper-case model text is shown
    in sentence case (presentation only).
* **Vite** dev dependency scanning is limited to Talktual's `index.html`,
  and `communication-engine/` is not watched (`cargo doc` writes HTML
  there).

## Alternatives
* A Node native addon or WASM build of CE inside the signaling server:
  much more work, and CE does not target WASM with threads.
* Talktual's server proxying audio to a remote CE: puts audio on a network
  path the user did not ask for. The bridge keeps it on the same computer.

## Consequences
Talktual can run end to end on CE recognition today on a desktop.
Talktual's default behaviour, tests and builds are unchanged. Phones need
HTTPS and network reachability (not tested). English only.
