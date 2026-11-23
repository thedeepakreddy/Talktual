# Phase 8.5 — Baseline and validation audit

Audit date: 2026-11-23.

## 1. Branch, commits and working tree (Git evidence)
| Item | Evidence |
| --- | --- |
| Phase 8 on `main` | `41cdb8f` merges PR #5 (`b03535e`) into `main`. The parent `735f147` is the PR #4 merge. The PR #5 merge was made at the owner's request at the start of Phase 8.5 |
| Phase 8 commits | `b34f356` pipeline · `1259df3` UI + E2E · `fdcf2f0` UI iteration + a11y · `b03535e` evaluation, device plan, report |
| Working branch | `claude/kind-archimedes-n6y11p`, fast-forwarded to `41cdb8f` (no force push), then Phase 8.5 commits |
| Working tree at audit start | clean (`git status` empty) |
| Rollback of Phase 8 | revert merge `41cdb8f` (`git revert -m 1 41cdb8f`), or hide the entry (`onAmbient` in `App.tsx`). Shared changes are behaviour-preserving for calls; the full call regression is green |

## 2. Trace: capture → recognition → translation → queue → playback, with the gates
| Stage | Code | Gate or bound | Verified by |
| --- | --- | --- | --- |
| Activation | `AmbientSession.start()` → `grantConsent()` → `begin()` | Separate ambient consent key; nothing is created before Allow | unit + E2E |
| Capability check | `begin()` | `secureContext`, `webSpeech`, offline → paused | unit + E2E |
| Recognition plan | `startCapture()` → `policy.plan('asr')` → `beginRemote('asr')` gate, or `processLocally` | Phase 6 policy; gate revocation pauses | unit + E2E (On-device) |
| Capture | `deps.createAsr(from, 'continuous')` → `lib/speechToText.ts` Web Speech adapter (`continuous`, `interimResults`, auto-restart, finalize timeout) | `abort()` + `capture.release()` on every stop | unit (fake), E2E (scripted Web Speech fake) |
| Segmentation | `StablePrefixSegmenter` | stable for 700 ms and ≥ 3 updates and ≥ 6 words; remainder-only finals; duplicate and filler drop | unit + evaluation |
| Translation | `privacy/routing.ts` `routedTranslate()` → device runtime and/or `/api/translate` | plan + execution gate + revocation; max 2 in flight, 8 waiting | unit + E2E (wire audit) |
| Server translation | `signaling-server/server.js` `/api/translate` → Gemini / DeepL / MyMemory | validation, rate limit, timeout; **cache now TTL-bound (Phase 8.5 fix)** | server tests |
| Ordering | `flushSpeech()` cursor | strict commit order; late (> 9 s) shown, not spoken | unit + evaluation |
| Queue and playback | `PlaybackController` (`lib/textToSpeech.ts`) | FIFO, max queue, `notAfter` stale drop, `cancelAll`; route resolved at start (device/cloud) | unit |
| Half-duplex | `onPlayback('started')` → `stopCapture('half-duplex')` | capture off while our speech plays; speech waits for a room pause | unit + E2E |
| Teardown | `pause()`, `stop()`, `dispose()`, visibility, offline, mic loss | abort + release; never re-enables the mic | unit |

## 3. Capability matrix — what is real, mocked, stubbed or simulated in the evidence so far
| Component | In production | In Phase 8 evidence | Real-world status |
| --- | --- | --- | --- |
| Browser speech recognition (Chrome cloud ASR, Safari Apple ASR) | real browser API | **Mocked** (unit `FakeAsr`) and **simulated** (E2E: scripted `SpeechRecognition` fake injected into real Chromium) | **Not exercised** with real audio anywhere |
| Microphone capture and routing | OS/browser | **Not exercised.** Chromium fake media devices only | Unverified |
| Talktual server (`/api/translate`, `/api/tts`) | real | **Real** server process in E2E | Verified on loopback |
| Translation providers (MyMemory/Gemini/DeepL) | real HTTP | **Stubbed** (`makeFakeFetch`) | Provider quality and latency unmeasured |
| On-device translation (phrasebook en↔es; Chrome Translator API) | real | Phrasebook real (en↔es); Translator API absent on test hardware | hu↔en on-device unavailable here |
| Speech output (server TTS / browser voice) | real | **Fake** `speechSynthesis`; server TTS returns an error in E2E → browser fallback path | Audio output never heard |
| Bluetooth / AirPods | OS | **Not exercised** | Unverified |
| Phase 6 policy and routing | real code | **Real** code under test | Verified (unit, E2E wire audit) |
| UI (Phase 7 system) | real | **Real** renders in Chromium, phone/tablet/desktop emulation | Verified in emulation only |
| Latency | — | **Simulated**: final→text p50 8 ms (loopback, stub) | **Real latency unmeasured.** The 8 ms figure must not be extrapolated |

## 4. Claims re-checked against code and behaviour (details in `PRIVACY_AUDIT.md`)
| Phase 8 claim | Result |
| --- | --- |
| "Talktual doesn't record or store audio" | **True**: no audio buffers are kept by ambient code; recognition is the browser's |
| "The text … is never saved" | **Client: true** (memory only; tests assert storage contains only consent and mode). **Server: was not strictly true.** The translation cache kept up to 2,000 source→translation pairs, without expiry, for the life of the process, shared across users, and **advertised hits** (`X-Provider: cache`). **Fixed in 8.5:** TTL 2 min by default (configurable, 0 disables) and hits no longer advertised |
| "Never log transcripts" | **Client: true. Server: provider error bodies were logged** (up to 300 characters; they could echo user text). **Fixed in 8.5:** only provider and status are logged |
| "Web Speech takes no MediaStream" (Phase 8 discovery) | **Outdated for desktop Chrome.** `SpeechRecognition.start(audioTrack)` ships in Chrome desktop 135+ (MDN, caniuse). It is still unsupported in Safari and Chrome for Android, so the iPhone conclusion stands. See `AUDIO_ROUTING_RESEARCH.md` |
