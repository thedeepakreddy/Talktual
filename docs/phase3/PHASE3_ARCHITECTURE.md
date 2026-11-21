# Phase 3 Architecture — VoiceMirror

Builds on [Phase 2 architecture](../phase2/PHASE2_ARCHITECTURE.md). Decision rationale: [RESEARCH_AND_FEASIBILITY](PHASE3_RESEARCH_AND_FEASIBILITY.md).

## 1. Data flow

```mermaid
flowchart LR
  subgraph Speaker device
    MIC[mic tap<br/>only while capture is active<br/>and VoiceMirror opted in] --> AN[VoiceAnalyzer<br/>streaming, O(1) memory<br/>YIN pitch histogram + envelope]
    AN -->|on capture stop| Q[toDescriptor<br/>confidence ≥ 0.5 else withheld]
    Q -->|changed?| W[peer message<br/>t:'voice', descriptor]
    OFF[opt-out] -->|t:'voice', revoked| W
  end
  W -- data channel / relay (wire v1) --> V
  subgraph Listener device
    V[validate isVoiceWireMessage] --> C[VoiceMirrorController<br/>peer descriptor in memory]
    SEG[peer segment] --> PQ[PlaybackController queue<br/>item.voice = resolver]
    PQ -->|segment STARTS| R[peerHints → toRenderHints]
    R -->|browser voice| BS[utterance.pitch / rate]
    R -->|server TTS| TTS["/api/tts {register, speed}"]
  end
  TTS --> SRV[server: validate hint<br/>voice id from SERVER catalog<br/>voice_settings.speed]
```

## 2. Module boundaries

| Module | File | Responsibility | Depends on |
| --- | --- | --- | --- |
| Contracts | `src/voicemirror/types.ts` | `VoiceProfile` (on-device), `VoiceDescriptor` v1 (wire), `RenderHints`, `VoiceTier` | — |
| Analyzer | `src/voicemirror/analyzer.ts` | Streaming pitch/tempo statistics; `reset()`; fixed state size | — |
| Descriptor | `src/voicemirror/descriptor.ts` | Quantize, validate (strict), map to clamped render hints | types |
| Controller | `src/voicemirror/controller.ts` | Opt-in/out, tap lifecycle, publish on change, peer descriptor, reconnect resend, dispose; enum-only diagnostics | analyzer, descriptor |
| Browser tap | `src/voicemirror/browserTap.ts` | `VoiceTapPort` over the shared call mic (`getMic`) + `ScriptProcessorNode` | lib/audio |
| Disclosure | `src/voicemirror/disclosure.ts` | Purpose-specific consent text | — |
| Engine integration | `CommunicationSession.ts` | Owns the controller; calls it at capture start/stop, connect, peer message, enqueue, teardown | controller |
| Playback seam | `engine/ports.ts`, `lib/textToSpeech.ts` | `PlaybackItem.voice?: () => RenderHints \| undefined` resolved at start | types |
| Server | `signaling-server/server.js` | `VOICEMIRROR_ENABLED`, `VOICEMIRROR_VOICE_CATALOG`, hint validation, `voiceHints` capability | — |

`src/voicemirror/*.ts` is plain erasable TypeScript, so the **same code** runs in the browser and in the Node evaluation harness (`node --experimental-strip-types`).

## 3. Contracts

### 3.1 `VoiceDescriptor` v1 (the only voice data that leaves a device)
```ts
{ v: 1, pitchStep: 0..44 (even), pitchRange: 'flat'|'normal'|'lively', tempoPct: 80..125 (step 5) }
```
* Quantized so that many voices share each descriptor (2-semitone pitch bins, 3 range classes, 5% tempo steps). It isn't an identifier and can't be inverted into audio.
* Validator rejects any other version, value, step or extra key (e.g. an `embedding` field). Covered by tests.

### 3.2 Peer message (wire v1, additive)
```ts
{ v?: 1, t: 'voice', descriptor }   // share / update
{ v?: 1, t: 'voice', revoked: true } // delete my data now
```
Phase 2 peers reject unknown `t` in `isPeerMessage`, so they ignore it. Builds without the flag count and ignore it.

### 3.3 Render hints (listener)
`register` (low < 145 Hz ≤ mid ≤ 200 Hz < high), `browserPitch` (damped ½-semitone mapping, clamped 0.6–1.6), `browserRate` 0.8–1.25, `ttsSpeed` 0.8–1.2, `targetF0Hz`.

### 3.4 Server hint
`POST /api/tts { text, lang, voice?: { register, speed } }`. Accepted only when `VOICEMIRROR_ENABLED=1`. Strictly validated (400 on any extra field, bad enum or out-of-range speed). **The voice ID always comes from server config** (`VOICEMIRROR_VOICE_CATALOG`, else the default voice); a client can't choose a voice ID.

## 4. Lifecycle and state

| Event | VoiceMirror behaviour |
| --- | --- |
| Session start | Controller created only if `voiceTap` dep exists (build flag). Sharing **off** |
| User toggles on | Disclosure sheet; nothing happens until "Share voice style" |
| Capture starts (PTT press resolved / hands-free listening) | Tap opens (separate getUserMedia consumer); analyzer accumulates across turns |
| Capture stops (release + flush, abort, mic off, peer away, reconnecting, playback pause) | Tap closes; descriptor computed; sent only if confident **and** changed |
| Not enough speech yet | Withheld (`descriptor-withheld`); listener uses default voice |
| (Re)connected | Current descriptor re-sent (the transport outbox deliberately keeps only segments + hello) |
| User toggles off (revocation) | Tap closed, analyzer and sent descriptor discarded, `revoked` sent |
| Peer revokes | Peer descriptor deleted; queued, not-yet-started segments resolve to default |
| Session ends (any terminal phase / dispose) | `controller.dispose()`: everything cleared; later input ignored |

The controller adds no new states to the Phase 2 machines and changes no Phase 2 transitions. Segment IDs, ordering, dedupe and buffer bounds are untouched (tests).

## 5. Bounded resources
* Analyzer state: fixed (≈ 1.3 KB pitch histogram + one 40 ms frame), independent of talk time (test).
* Descriptor: 58 bytes JSON.
* No new queues. The playback item carries a resolver, not audio.

## 6. Privacy boundaries
* Raw audio: analyzer only, never buffered beyond one frame, never transmitted for VoiceMirror.
* `VoiceProfile` numbers: never leave the controller, never logged.
* Diagnostics: `voicemirror` events carry only a `status` enum (test asserts exact keys).
* Server: receives `{register, speed}` only with TTS requests (no descriptor, no profile). Logs unchanged (no text, no hints).

## 7. Higher tiers (designed, not built)

| Tier | Where it plugs in | Extra requirements |
| --- | --- | --- |
| `stock-selection` (embedding → nearest catalog voice) | Analyzer gains an on-device encoder; descriptor gains a catalog *index* (not the embedding); server catalog grows | Model packaging, catalog embeddings from provider audio, re-assessment of identifiability |
| `consented-clone` (zero-shot TTS/VC) | Server renderer keyed by a short-lived, per-call voice handle; `PlaybackItem.voice` resolver returns that handle | GPU/provider, consent and verification flow, retention + deletion API, abuse controls, owner/legal approval |

`VoiceTier` is in the contracts so these can be added without changing the session or playback seams.
