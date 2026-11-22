# Phase 6 — M6.0 Audit and Feasibility

Branch `claude/kind-archimedes-n6y11p` @ `f393bde` (Phase 5). Baseline: 389 client tests, 58 server tests, 6 browser suites green.

## 1. Data inventory (verified in code)
| Data | Produced | Leaves the device to | Retained / cached | Logged | Cleared |
| --- | --- | --- | --- | --- | --- |
| Microphone audio | `CapturePort` while talking | **Browser Web Speech** → the browser vendor's cloud recognizer (Chrome/Edge: Google/Microsoft; Safari: Apple); or **server STT** → Talktual server → ElevenLabs | not stored | no | capture released on stop |
| Transcript text (finals, partials) | recognizer | Talktual server → translator (MyMemory/Gemini/DeepL); peer (data channel/relay) | session transcript ≤ 500 records (memory); **client translation cache: 500 entries, module-level, survives across calls in the same page**; **server translation cache: 2000 entries keyed by source text (server memory)** | no (allowlist redactor) | transcript on dispose; caches **not** cleared (fixed in Phase 6 for the client) |
| Live-caption translation | peer partials | Talktual server → translator | client cache | no | — |
| Phase 4 drafts | held fragments | translator + peer (`conv1`) | no | no | retract |
| TTS | translated text | **server TTS** → Talktual server → ElevenLabs, tried first for every segment; fallback **browser speechSynthesis**, which may use a *network* voice (`localService=false`, e.g. Chrome "Google …" voices) | no | no | cancel |
| Phase 5 context | finals | none (assist codes only) | memory, bounded | no | reset/disable/end |
| VoiceMirror | off by default | — | — | — | — |
| Consent | user | none | localStorage per route | no | revoke |

**Policy today:** there is no notion of processing location. Every network-backed function is used whenever available, and consent is per ASR route only.

## 2. Local runtime feasibility on the test hardware (real probe)
Evidence: `e2e/probe-local-capabilities.mjs` → `docs/phase6/probe-local-capabilities.json`.

Test hardware: Linux x86_64 container, 4 vCPU Xeon @ 2.3 GHz, 16 GB RAM, **no GPU**, Chromium 157 headless.

| Candidate | Probe result here | Licence / distribution | Install / size | Verdict |
| --- | --- | --- | --- | --- |
| Chrome on-device Web Speech (`processLocally: true`) | **`available()` crashed the renderer** (cloud `available()` → "available") | platform | language pack download (`install()`) | **Not usable here.** The adapter ships **disabled by default**; enabling it needs a build flag plus real-device verification |
| Chrome built-in Translator API | `availability()` **never answered** (3 s) | platform | model download on first use | Adapter implemented, **used only if the platform reports `available`** (time-bounded probe, never triggers a download). Unverified here |
| speechSynthesis local voices (`localService`) | **0 voices** (none at all) | platform/OS | OS voices | Adapter implemented (local voices only). Unverified here |
| WebGPU | API present, **no adapter** | — | — | no GPU inference |
| WASM models (Whisper-tiny ASR, Opus-MT en↔es, Piper TTS) via onnxruntime-web / transformers.js | WASM available; models **not present** | MIT / Apache-2.0 / CC-BY-4.0 (model-specific) | ~40–300 MB downloads plus a new npm dependency | **Blocked by authorization** (no model or dependency downloads). Recommended follow-up |
| Python onnxruntime (container) | installed, no models | — | — | server-side only; not on-device |
| espeak-ng (container) | installed | GPL-3.0 | system binary | server-side only (and the Phase 3 eval tool); not on-device |
| **Bundled phrasebook translator (en↔es)** | pure TypeScript, no model | own content (project licence) | ~20 kB in bundle | **Feasible and verifiable** → implemented as the first verified local translation capability, with honest, measured coverage |
| Phase 5 ACI | on-device rules | own | in bundle | already local (verified Phase 5) |

## 3. Feasibility matrix (planned before implementation)
| Feature | en→es | es→en | Other pairs | Device: test hardware | Device: Chrome desktop w/ models | Device: Safari/iOS |
| --- | --- | --- | --- | --- | --- | --- |
| Translation (phrasebook, local) | ✅ common phrases only | ✅ common phrases only | ❌ | ✅ verifiable | ✅ (same code) | ✅ (same code) |
| Translation (Translator API, local) | runtime-gated | runtime-gated | runtime-gated | ❌ not available | unverified (needs device) | ❌ (no API) |
| Speech recognition (local) | runtime-gated, off by default | same | same | ❌ (crash) | unverified | ❌ (no web API for on-device) |
| Speech output (local voice) | runtime-gated | runtime-gated | runtime-gated | ❌ (no voices) | unverified | unverified (iOS voices are local) |
| Typed input (no ASR needed) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Conversation intelligence (Phase 5) | ✅ | ✅ | partial (rules en/es) | ✅ | ✅ | ✅ |
| Calls without any network | ❌: both peers need the signaling server, so a fully offline call is impossible | | | | | |

**Consequence for "offline":** a call needs network. *Offline/degraded* in Talktual therefore means **the providers or the server are unreachable while the call (peer data channel) survives, or the user chose local-only**. In that state, the verified local capabilities (typed input, phrasebook translation, intelligence, transcript) keep working, and everything else is shown as unavailable.

## 4. Plan
`src/privacy/`: processing modes and the central policy gate with audit. `src/local/`: runtime contracts, registry and bounds, phrasebook runtime, gated platform runtimes. Integration: session routes translation, captions, ASR and TTS through the policy; sender privacy flag (`priv1`) so the receiver keeps local-only content on-device; typed input; network monitor for offline/degraded; client cache clearing. UI: privacy sheet (mode, capability rows, data-flow text), offline/degraded chip, typed input. Evaluation: blind phrasebook test set (separate agent), latency and memory on this hardware, offline and network E2E with route interception.
