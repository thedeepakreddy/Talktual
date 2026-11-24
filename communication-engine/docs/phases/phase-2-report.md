# Phase 2 report — Audio foundation

Date: 2026-11-24 · Branch: `deepak/gallant-hamilton-wegi2i` (from `main` @
`0ad2dc6`). Verified on Linux x86_64 (cloud container, no audio hardware),
Rust 1.97.0 and 1.85.0 (MSRV).

**Hardware status: no audio hardware was available or tested.** The only
real backend run was cpal's ALSA host against ALSA's virtual `null` PCM.
Nothing here is a latency, stability, battery or device claim.

## 1. Repository baseline and platform findings

See [phase-2-baseline-and-plan.md](phase-2-baseline-and-plan.md). Phase 1
was merged (PR #8); Talktual's audio is browser-only; the web-vs-native
decision (DR-8.5-1) is open, so CE targets native hosts (iOS, Android,
desktop) and leaves the browser to Talktual's existing code. Baseline gates
all passed before any change (CE 196 test results; Talktual tsc, 537, 60,
build).

## 2. Backend research and decision

ADR-0010: **cpal 0.18.2** behind a CE-owned `AudioBackend` trait, chosen
over per-platform crates (4× adapters), cubeb (C++ build), PortAudio (C
library, stale bindings) and miniaudio/tinyaudio (stale/limited). Apache-2.0,
active releases (Feb/Jun/Aug 2026), MSRV 1.85, compiles for every target
checked. Its gaps (no hot-plug, no permission query, no channel layout) are
handled in CE (polling + diff, deny-by-default permission with host
override, no invented layouts). `rtrb` 0.4 provides the wait-free SPSC rings.

## 3. Architecture and contracts

[audio.md](../architecture/audio.md), ADR-0011/0012/0013.

* `ce-core::audio` — `SampleFormat`, `ChannelLayout`, `StreamFormat`,
  `SupportedFormatRange` (+ sanitization), `AudioDeviceDescriptor`,
  `DeviceCatalog`, `AudioDeviceId` (opaque, stable per runtime),
  `PlatformPermission`, `StreamRequest`/`FormatPreference`/`ChannelMode`,
  `negotiate()`, `ResolvedFormat` (requested, device, delivered, buffer
  requested/actual, adjustments), `AudioStreamState` table, `StreamInfo`,
  `StreamCounters`, `AudioStreamStatus`, `TerminalReason`, `AudioFrameMeta`,
  `AudioBlock`, `Discontinuity`, `evaluate_capture_authorization()`,
  `AudioEvent` (17 kinds), `AudioConfig`. New error categories `device`,
  `unauthorized`; new commands `UpdateConsent`, `ListAudioDevices`,
  `Start/Stop/CloseAudioStream`, `ListAudioStreams`.
* `ce-audio` — `AudioBackend`/`BackendStream` (callback-quiescence contract),
  `CaptureEndpoint`/`PlaybackEndpoint`/`FaultReporter` (callback side),
  `RawSample` conversions, stream workers, `CaptureReader`/`PlaybackWriter`,
  `PermissionOverride`, `cpal_backend` (feature `cpal`).
* Runtime — audio state owned by the core (child module `core/audio.rs`):
  catalog with diffing, gate, stream lifecycle, signal handling, deferred
  revocation, housekeeping, session/shutdown cleanup. Facade: `audio_backend`
  on the builder; `audio_devices`, `update_consent`, `open_capture`,
  `open_playback`, `start_audio`, `stop_audio`, `close_audio`,
  `audio_streams`.

## 4. Implemented functionality

Device discovery and capability reporting; format negotiation with exact
requested-vs-resolved reporting (incl. actual buffer size); capture and
playback with explicit validated lifecycle; normalization to interleaved f32
(+ opt-in mono downmix); per-stream sequence numbers, sample-clock positions,
monotonic and device timestamps; bounded rings and queues with documented
per-path policies; discontinuities placed at the exact frame; device
removal/default change/reroute handling without silent restart;
consent ∧ permission capture gate with denial, staleness and revocation;
failure supervision (timeouts, panics, stuck workers); observability without
audio content; metrics; health integration.

## 5. Files and changes

| Path | Change | Lines |
| --- | --- | --- |
| `crates/ce-core/src/audio/` (9 files) | new audio contracts + unit tests | 2,188 |
| `crates/ce-core/src/{command,config,error,event,health,ids,lib}.rs` | commands, config, categories, payload/topic, health/metrics, `AudioStreamId` | edits |
| `crates/ce-audio/` (new crate) | backend boundary, rt endpoints, conversion, io, workers, cpal adapter | 2,799 |
| `crates/communication-engine/src/runtime/core/audio.rs` | orchestration in the core | 1,263 (dir) |
| `crates/communication-engine/src/{lib.rs,runtime/core.rs,runtime/metrics.rs}` | facade API, core wiring, metrics | edits |
| `crates/ce-testkit/src/fake_audio.rs` | fake backend with fault injection | 726 |
| `crates/communication-engine/tests/audio_*.rs` (7 files) | integration/fault/concurrency/real-backend tests | 2,120 |
| `crates/communication-engine/examples/audio_smoke.rs`, `audio_probe.rs` | smoke (CI gate), measurement probe | 449 |
| `docs/architecture/audio.md`, ADR-0010–0013, updates to 0009, realtime, privacy, talktual-integration, ARCHITECTURE, READMEs | documentation | — |
| `docs/phases/phase-2-*.md` | baseline/plan, measurements, this report | — |
| `scripts/verify.sh`, `.github/workflows/communication-engine.yml` | audio smoke, ce-audio cross-checks, ALSA headers in CI | edits |

## 6. Talktual integration impact

None on the running app: no Talktual source changed; root README gained two
sentences; checks unchanged (tsc pass, 537, 60, build pass, re-run after the
change). CE audio is usable by Talktual only via a native shell or a
desktop/server host; the browser keeps Talktual's Web Audio path. Concept
mapping and integration paths: [talktual-integration.md](../architecture/talktual-integration.md).

## 7. Tests, commands and results (all executed; results observed)

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass (includes cpal adapter) |
| `cargo test --workspace --all-targets` | **171 passed**, 0 failed |
| `cargo test --workspace --all-features` | **175 passed** (+ cpal ALSA-null tests, tracing, doctest) |
| `RUSTDOCFLAGS=-D warnings cargo doc --all-features` | pass |
| `runtime_smoke`, `audio_smoke` examples | PASS, PASS |
| `cargo check` ios/android/windows/macos (all features incl. cpal) | pass (compile only) |
| `cargo check -p ce-core -p ce-audio --target wasm32-unknown-unknown` | pass |
| `cargo +1.85.0 check --workspace --all-targets --all-features` | pass |
| `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok |
| full suite incl. cpal, 20 consecutive runs | 0 failing runs |
| audio suites, 9 concurrent runs under 4 busy-loop CPU burners | 0 failing runs |
| Talktual `lint` / `test` / `test:server` / `build` | pass / 537 / 60 / pass |

New tests (74 default + 2 ALSA-null): contract units (formats, layouts,
sanitization, negotiation policy, lifecycle matrix, request validation,
frame arithmetic, authorization matrix and staleness, config) — 18;
`ce-audio` units (conversion round-trips/clipping/rounding/NaN/downmix,
endpoint overflow/gap markers/gating/underrun classification, faults,
queue policies and bounds) — 15; integration — lifecycle 8, authorization
8, devices 6, faults 9, queues 5, concurrency 5, cpal ALSA-null 2.

Failures found and fixed during the phase: worker struct not `Send` with a
backend stream slot (moved to a loop local, which also guarantees drop on
panic); 32-bit precision claim (corrected to ≤ 128 LSB, tested); reader info
missing post-open buffer size (now refreshed); invalidation discarding
already-captured frames (worker now drains before ending the data plane);
discontinuity counter not counting queue drops (found by the probe on the
real backend); several test races (fixed in tests, with the race documented).

## 8. Hardware testing evidence

**None.** No sound card, audio server or user device was available
(`/dev/snd` absent; no computer attached to the session). Real-backend
evidence is limited to cpal/ALSA on the virtual `null` PCM:
`tests/audio_cpal_alsa.rs` (enumeration, negotiation, open/start/stop/close,
callback-driven capture of zeros with every overflow gap flagged exactly,
playback consumption, unusable `default` device handled as a typed error)
and [phase-2-measurements.md](phase-2-measurements.md). Untested targets:
macOS, iOS, Android, Windows (compile-checked only), Linux with real devices,
every Bluetooth/USB route.

## 9. Deviations and rationale

1. **New crate `ce-audio`** instead of putting audio in `ce-core`/the runtime:
   keeps library-specific code and the real-time handoff out of the
   runtime-agnostic contracts, and keeps cpal optional (feature-gated).
2. **Opening a stream is a facade call, not a serializable `Command`**: it
   returns an in-process data-plane endpoint. Start/stop/close/list are
   commands.
3. **No resampling** (ADR-0012): rates are negotiated with the device and
   reported exactly; resampling waits for Phase 3 evidence.
4. **Platform permission is host-supplied** on non-ALSA platforms (cpal cannot
   query it); capture is denied until supplied. Per-platform permission
   integration is not implemented or verified.
5. **Worker threads, not Tokio tasks**, for streams (ADR-0011).
6. **Device names are personal data** and never leave the catalog.

## 10. Risks and limitations

* No hardware validation; latency, glitch rate, Bluetooth routes and battery
  are unknown. cpal's callback-quiescence-on-drop is verified only for ALSA.
* Poll-based draining adds ≤ `worker_poll_ms` latency; anchor accuracy of
  `timestamp` ≈ poll + one buffer.
* cpal has no hot-plug: device changes are detected only on notices
  (none from cpal), on request, at opens, or by optional polling.
* Linux binaries link LGPL libasound dynamically (distribution review).
* A backend call that never returns leaves a zombie thread (bounded, reported
  as unclean shutdown; cannot be killed safely).
* Timing marker / gap marker rings (64) can overflow under extreme callback
  rates; loss is then attributed to the next marker (position imprecise,
  count exact).
* Browser embedding of the runtime remains unsupported (Phase 1 limitation).

## 11. Phase 2 acceptance checklist (spec §13)

- [x] Baseline and Phase 1 boundaries documented; existing changes preserved
- [x] Backend selection evidence-based, license-checked, recorded (ADR-0010)
- [x] Typed, backend-independent audio contracts (`ce-core::audio`)
- [x] Device discovery and capability reporting through the public facade
- [x] Capture/playback lifecycle explicit and validated (table + tests)
- [x] Requested and resolved formats represented accurately (incl. actual buffer)
- [x] Buffers and queues bounded with documented overflow policy
- [x] Timestamp, sequencing, correlation and discontinuity semantics implemented and tested
- [x] Callback/control-plane separation and supervision tested and documented
- [x] Permission/consent enforced as capture gates; denial and revocation tested
- [x] Device changes, invalidation, failures and cleanup handled and tested
- [x] Fake backend and meaningful unit/integration/fault tests
- [x] Public API smoke example runs (`audio_smoke`, CI gate)
- [x] Phase 1 gates pass; Talktual checks at baseline
- [x] fmt, Clippy `-D warnings`, tests, docs, license/security checks pass
- [x] Hardware results honest; untested targets identified (§8)
- [x] Documentation and ADRs updated
- [x] Completion report and Phase 3 readiness delivered

All criteria are met **as written**. The criterion on hardware is met by
honestly reporting that none was tested — which is also the main gap.

## 12. Phase 3 readiness

**Ready to start Phase 3 work that consumes audio through CE contracts**
(the frame contract, timestamps, discontinuities and consent gate are in
place and tested), **with one blocking prerequisite for any production
decision**: run `audio_probe` and the capture/playback paths on the real
target devices (at minimum iOS with AirPods per DR-8.5-1, Android, and one
desktop OS) and implement host-side permission supply (`PermissionOverride`)
on each. Phase 3 should also decide resampling (16 kHz for ASR) from those
measurements and add loopback latency measurement before any latency claim.
