# Phase 6 Architecture — Privacy, Local Intelligence and Offline

```
                       ┌──────────── ProcessingPolicy (src/privacy/policy.ts) ────────────┐
 user mode ───────────▶│ mode: cloud | hybrid | local-only   network: online|offline|      │
 NetworkMonitor ──────▶│ plan(task, ctx) → ordered steps [device|cloud] + reason          │
 (online events +      │ beginRemote(task) → execution-time gate; aborted on revocation   │
  provider failures)   │ enum-only audit (≤ 200)                                           │
                       └───────────────▲──────────────────────────▲────────────────────────┘
                                       │ every inference step      │
 CommunicationSession ─────────────────┴──────────────────────────┴───────────────────────
   ASR ........ plan('asr')       → on-device recognizer (processLocally) | cloud recognizer | none (fail closed, typing offered)
   translate .. routedTranslate   → RuntimeRegistry.translate (phrasebook / browser Translator) | deps.translate (server → provider)
   captions ... plan('caption')   → registry | translateCaption (gated) | raw text
   TTS ........ item.ttsRoute()   → device-only | device-first | cloud-first  (resolved when the item STARTS)
   wire ....... hello.caps 'priv1'; segment/partial `priv: 'device'` (local-only sender) → receiver keeps those words on-device
 RuntimeRegistry (src/local/registry.ts): discovery (time-bounded), ≤ 2 loaded, concurrency 1, queue ≤ 4, 1.5 s call bound,
   60 s idle unload, unloadAll (memory pressure/clear), output validation
   ├─ PhrasebookRuntime (bundled, en↔es)              — verified on test hardware
   ├─ BrowserTranslatorRuntime (Chrome Translator API) — used only if the platform says 'available'; never downloads
   ├─ LocalVoiceRuntime (speechSynthesis localService) — used only for voices marked on-device
   └─ OnDeviceAsrRuntime (Web Speech processLocally)   — DISABLED by default (crashed the tab on test hardware)
```
Phase 2 still owns lifecycle, transport, cancellation and playback. Phase 4 still owns segmentation, interruption and overlap. Phase 5 still owns notes and context; ACI was already on-device and is unchanged.

## 1. Mode semantics (precise)
| Mode | Plan per task (deviceAvailable = an on-device runtime exists) | Never |
| --- | --- | --- |
| **Cloud-assisted** (default) | cloud first; on-device if cloud is unreachable or fails (`device-fallback`) | — |
| **Hybrid** | on-device first; cloud only if on-device cannot handle it and cloud is permitted | cloud ASR without consent |
| **On-device only** | on-device only; **no step ⇒ unavailable (fail closed)** | any cloud step for ASR, translation, captions or TTS; `beginRemote` throws `PolicyBlockedError` |
| *Offline* (derived: browser offline) | on-device only, in every mode; in-flight remote work aborted | cloud |
| *Providers unreachable* (derived: 2 consecutive network failures, 30 s cooldown) | cloud skipped; on-device used | retry loops |
| *Degraded* (derived) | any function unavailable in the current state is listed as such in the UI | presenting cloud as local |

* **Peer request.** A local-only sender marks segments and partials `priv: 'device'` (priv1 peers only). The receiver then applies on-device-only routing to that content: fix-up translation, captions and TTS.
* **Mid-session change.**
  * Switching to on-device only, or going offline, aborts tracked remote work: an in-flight translation (which then falls to the next allowed step: on-device, or sent untranslated), cloud ASR (stopped), drafts, and playback (stopped).
  * Revoked remote work is **never resumed**.
  * Client caches are cleared on every mode change.
* **Not inference.** Signaling, transport and the capability fetch to the Talktual server are not inference. They are allowed in every mode, because the call needs them (documented in the UI text).

## 2. Contracts
* `src/local/types.ts`:
  * `RuntimeInfo`: id, task, languages, model {name, version, license, sizeKB, install}, platform, readiness, constraints, quality, resources, verification (`verified-test-hardware` | `platform-reported` | `unverified`).
  * `TranslateRuntime`: discover/load/unload/translate/dispose.
  * `VoiceRuntime`, `AsrRuntime`.
* `src/privacy/policy.ts`:
  * `ProcessingMode`, `InferenceTask`, `RoutePlan`, `RouteReason`, `AuditEntry`, `PolicyBlockedError`.
  * `PlaybackItem.ttsRoute?: () => TtsRoute`.
  * `AsrFactory(lang, mode, cb, opts?: { processLocally })`.
  * `SessionDeps.processing?: boolean | ProcessingOptions` (off when absent, giving exact Phase 5 behaviour) and `SessionDeps.clearCaches`.
* Session API:
  * Commands: `setProcessingMode`, `clearPrivateData`, `sendText`.
  * Snapshot: `processing {available, mode, network, capabilities[], degraded, runtimes[], peerPrivate}`.
  * Records: `processedOn`, `typed`, `privateRequest`.

## 3. Capability matrix (verified status)
Test hardware: Linux x86_64 container, 4 vCPU Xeon 2.3 GHz, 16 GB RAM, no GPU, Chromium 157 headless.

| Feature | Runtime | Languages | Test hardware | Real devices | Mode availability |
| --- | --- | --- | --- | --- | --- |
| Translation (offline) | Bundled phrasebook | en↔es, covered phrases only (53/85 of in-scope blind phrases; 0/15 out-of-scope) | ✅ **verified** (unit, E2E, blind eval, real latency/memory) | same code (pure JS); not run on phones | all modes |
| Translation (on-device neural) | Chrome Translator API | pairs the browser reports | ❌ not available (`availability()` never answered) | **unverified** | used only if reported `available` |
| Speech recognition (on-device) | Web Speech `processLocally` | per browser | ❌ crashed the renderer → **disabled by default** | **unverified** | off (flag `VITE_ONDEVICE_ASR=1`) |
| Speech output (on-device) | OS voices (`localService`) | per OS | ❌ 0 voices | **unverified** (E2E uses *simulated* voices) | used only for on-device voices |
| Typed input | none needed | any | ✅ verified | ✅ (plain HTML input) | all modes |
| Conversation assistance (Phase 5) | rules | en/es | ✅ verified (Phase 5) | — | all modes |
| Cloud ASR / MT / TTS | browser vendor / server providers | all | (stubbed in tests) | — | cloud, hybrid |
| **Calls without network** | — | — | ❌ impossible: signaling needs the server | — | — |

**No offline speech recognition or speech output is claimed on any platform.** Offline translation is claimed only for en↔es phrasebook coverage.

## 4. Data flow and threat model
**Trust boundaries:**
1. This device: browser and app memory.
2. The other participant's device.
3. The Talktual server (signaling, proxy).
4. Third-party providers (browser vendor speech service, MyMemory/Gemini/DeepL, ElevenLabs).

| Mode | Leaves boundary 1 → 2 | → 3 | → 4 |
| --- | --- | --- | --- |
| On-device only | your words (typed or on-device recognized) + translation; `priv` request | signaling only (no content when P2P; relay carries messages if P2P fails) | **nothing** (E2E: 0 `/api/translate`, `/api/tts`, `/api/stt` requests from either page; no recognizer started) |
| Hybrid | same | + text for phrases the device can't translate; TTS when no on-device voice | via server: those texts; browser cloud ASR audio (with consent) |
| Cloud-assisted | same | text, TTS text, server-STT audio | translator, TTS, STT providers; browser speech service |

| Threat | Mitigation | Residual |
| --- | --- | --- |
| Silent cloud fallback in local-only | Single policy; execution-time `beginRemote` gate; revocation aborts in-flight work; tests assert no cloud step for every task × context; wire-level E2E audit | A future code path that calls a provider without the policy. Mitigated by routing all deps through session helpers; reviewable via `beginRemote` call sites |
| Peer re-processes my words in the cloud | `priv: 'device'` honoured by priv1 peers | An old or modified peer can ignore it (disclosed) |
| Relay sees content | unchanged from Phase 2 (relay used only when P2P fails) | owner config |
| Network voices posing as local | only `localService` voices in device routes; voice assignment failure → silence | relies on the browser's `localService` flag |
| Stale or replayed remote work after reconnect | revoked work not resumed; Phase 2 dedupe; E2E: exactly one delivery after offline/online | — |
| Caches / retention | client translation cache cleared on session end and mode change; ACI context cleared; audit cleared; only the mode setting persists | **Server translation cache (2000 entries, memory) unchanged**: owner decision (server not modified in Phase 6) |
| Diagnostics leak | routes recorded as enums; leak tests | — |
| Malicious runtime output | output validation (non-empty, ≤ 4× input + 60, no control chars), timeouts, concurrency and queue bounds | — |
| Crash-prone platform API | on-device ASR probe disabled by default; all discovery time-bounded | other platform bugs |

## 5. Rollback
* Build with `VITE_PROCESSING_MODES=0` for exact Phase 5 behaviour (rollback E2E). The engine is off when `processing` is absent.
* Per call: choose "Cloud-assisted" (Phase 5 routing plus the on-device fallback).
* Code: revert `ab5628d..HEAD`. The wire is compatible: `priv` and `priv1` are optional.
