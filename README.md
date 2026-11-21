# Talktual

Talktual is a peer-to-peer walkie-talkie that translates as you speak. Two people pick their own language, hold a push-to-talk button, and each hears the other in their own language — no shared language, no menus, no per-message setup.

Each side chooses from 31 languages independently, on the call screen, and can switch mid-conversation: the change is announced over the live data channel, so the other person's next sentence arrives already retargeted without reconnecting.

## How it stays fast

The naive version of this app records the whole push, uploads it, transcribes it, translates it, synthesizes it, and only then plays anything. Every stage is serial and none of it starts until the speaker lets go, which lands somewhere north of three seconds of silence per turn.

Talktual moves the work *underneath* the push instead:

1. **Transcription happens while you hold the button.** The Web Speech API streams interim text continuously and finalizes each phrase at natural pauses. A long sentence becomes several segments.
2. **Each finalized segment is translated and sent immediately** — while the speaker is still talking. On a multi-sentence turn the listener starts hearing the translation before the speaker has finished.
3. **Speech synthesis streams.** The server requests raw PCM from ElevenLabs' streaming endpoint and pipes it straight through; the browser schedules each chunk onto the audio clock as it arrives, so playback begins on the first chunk rather than the last.
4. **Text travels over a WebRTC data channel**, peer to peer, skipping a server round trip.

What's left after release is a translation call plus time-to-first-audio, typically a few hundred milliseconds with a fast translation provider configured.

The listener also sees a live caption of the speaker's interim text as it's produced, which covers the remaining delay with something to read.

## Architecture

### Frontend (React + Vite)

- **`lib/speechToText.ts`** — streaming recognition via Web Speech, with phrase segmentation gated by the push-to-talk button (plus a short trailing grace period so the last word isn't clipped). Browsers without Web Speech fall back to recording the push and posting it to ElevenLabs Scribe on release: correct, but roughly a second slower.
- **`lib/textToSpeech.ts`** — streamed PCM playback scheduled through the Web Audio API, with an ordered queue so multi-segment turns play back in the order they were spoken. Falls back to MP3, then to the browser's own `speechSynthesis`, so a call never goes silent.
- **`lib/webrtc.ts`** — data-channel transport with a language handshake. The signaling socket doubles as a fallback transport, so a call still works when P2P can't be established.
- **`lib/translate.ts`** — proxy-backed translation with a local cache and in-flight deduplication.
- **`lib/audio.ts`** — one microphone stream and one AudioContext per call, plus the user-gesture unlock mobile browsers require before they'll play audio.

### Backend (Node + Express + ws)

- **Signaling server** routing SDP offers, answers, and ICE candidates between two peers.
- **`POST /api/tts`** streams from ElevenLabs (`eleven_flash_v2_5` by default) and pipes the bytes through without buffering the clip.
- **`POST /api/translate`** uses the fastest configured provider and falls back to MyMemory, which needs no key.
- **`POST /api/stt`** proxies ElevenLabs Scribe for the compatibility path.
- **`GET /api/capabilities`** tells the client which providers are live, so it only enables live translated captions when the translator is cheap enough to run on interim text.

All API keys stay on the server; the browser never sees one.

## Transport and reachability

Text goes peer to peer over a WebRTC data channel when ICE succeeds. Behind symmetric NAT — common on mobile carriers and corporate wifi — STUN alone isn't enough, so set `VITE_TURN_URL` if you need P2P there. Without TURN those calls fall back to relaying text through the signaling server, which is unnoticeable in practice since only small text messages cross the wire. The call header shows which transport is live.

## Getting Started

### Prerequisites

- Node.js 20+ (verified on Node 22.23.5 / npm 10.9.9)
- Optional: an ElevenLabs API key (natural voices) and a Gemini or DeepL key (fast, high-quality translation)

The app runs with no keys at all — it falls back to the browser's built-in voice and the keyless MyMemory translator. Without an ElevenLabs key, browsers that lack the Web Speech API (e.g. Firefox) cannot transmit speech; the call screen says so up front.

### Backend

```bash
cd signaling-server
npm ci
cp ../.env.example .env   # then fill in the keys you have
npm start
```

Runs on `http://localhost:8080`. On boot it logs which providers it found and warns about unset hardening options.

### Frontend

```bash
npm ci
echo 'VITE_SIGNALING_URL=ws://localhost:8080' > .env.local
npm run dev
```

Speech recognition requires a secure context: `localhost` is fine, but any other host needs HTTPS.

## Testing

| Command | What it runs |
| --- | --- |
| `npm run lint` | TypeScript type-check (`tsc --noEmit`) |
| `npm test` | Client unit/component tests (Vitest + jsdom) |
| `npm run test:server` | Server API + signaling tests (`node:test`, providers stubbed) |
| `npm run build` | Production build |
| `npm run test:all` | All four of the above — the CI gate |
| `npm run test:e2e` | Two-browser Chromium smoke test against the production build and a real server instance (needs Playwright + Chromium installed) |
| `node e2e/perf.mjs` | Server latency / memory benchmark |

No test makes a live call to a translation, speech or TTS provider: providers are stubbed at the server's `fetch`, and the E2E test scripts the browser's speech recognizer and records speech synthesis. See [docs/phase1/TEST_REPORT.md](docs/phase1/TEST_REPORT.md).

## Configuration

See [.env.example](.env.example) for the full list. The ones that matter most:

| Variable | Where | Effect |
| --- | --- | --- |
| `VITE_SIGNALING_URL` | frontend | Signaling server; its http(s) form is also the API proxy |
| `ELEVENLABS_API_KEY` | backend | Natural TTS voices and STT for browsers without Web Speech |
| `GEMINI_API_KEY` / `DEEPL_API_KEY` | backend | Fast, good translation, and enables live translated captions |
| `ELEVENLABS_TTS_MODEL` | backend | Defaults to `eleven_flash_v2_5`; `eleven_multilingual_v2` sounds better but costs about a second |
| `ALLOWED_ORIGINS` | backend | **Set in production.** Browser origins allowed to use the API and signaling |
| `RATE_LIMIT_PER_MINUTE` / `TRUST_PROXY` | backend | **Set in production.** Per-IP limit on paid endpoints; trust the platform proxy for client IPs |
| `VITE_TURN_URL` | frontend | TURN server for P2P behind symmetric NAT (note: `VITE_*` values are public) |

## Deployment

- **Frontend:** static site (Vercel, Render). Vite inlines `VITE_*` variables at build time, so rebuild after changing them. Never put a secret in a `VITE_*` variable.
- **Backend:** Node web service. Serves the WebSocket signaling endpoint and the HTTP API proxy on one port. Set `ALLOWED_ORIGINS`, `RATE_LIMIT_PER_MINUTE` and (behind a proxy) `TRUST_PROXY=1`.

## Limitations and troubleshooting

- **"Speech recognition isn't available…"** — the browser has no Web Speech API and the server has no `ELEVENLABS_API_KEY`. Use Chrome/Edge/Safari or configure the key.
- **"Translation unavailable — …"** — every configured translator failed or timed out (8 s). The original words are sent and clearly marked; nothing is passed off as a translation. Check the server log.
- **"No one is waiting on that channel"** — the joiner waited 12 s without the host. Check the code; codes expire after 10 minutes unpaired.
- **Call shows "Disconnected"** — both the signaling socket and the P2P channel dropped. There is no automatic reconnect yet: leave and start a new channel.
- **Privacy:** while a call is open on Web Speech browsers, the recognizer listens continuously (results are only *used* while the button is held), so audio goes to the browser's speech service for the duration of the call. See [docs/phase1/SECURITY_REPORT.md](docs/phase1/SECURITY_REPORT.md).
- Verified browsers: headless Chromium only. See [docs/phase1/COMPATIBILITY_REPORT.md](docs/phase1/COMPATIBILITY_REPORT.md).

## Phase 1 audit

Audit, repairs and release-gate evidence live in [docs/phase1/](docs/phase1/STABILIZATION_REPORT.md).
