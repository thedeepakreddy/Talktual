# Phase 3 report — Audio intelligence

Date: 2026-11-24 · Branch: `deepak/gallant-hamilton-wegi2i` (on top of the
Phase 2 commit `fd18b68`). Verified on Linux x86_64 (cloud container,
4 vCPU, no audio hardware), Rust 1.97.0 and 1.85.0 (MSRV).

**Hardware status: no audio hardware was available or tested.** All audio
came from the test kit's fake backend and from synthetic TTS fixtures.
**No human recordings were used.** Nothing here is a latency, accuracy-on-
real-speech, device, battery or thermal claim.

## 1. Repository baseline

[phase-3-baseline-and-plan.md](phase-3-baseline-and-plan.md). The Phase 2
commit was re-tested in a separate worktree before the Phase 3 changes were
relied on: 172 tests (default features) and 175 (all features), 0 failures.
Talktual: tsc pass, 537 client tests, 60 server tests, build pass.

## 2. Research

[phase-3-research.md](phase-3-research.md), ADR-0015. Reviewed CE-native
detectors, webrtc-vad 0.4, earshot 1.2.2, nnnoiseless 0.5.2, Silero/TEN.
Benchmarked webrtc-vad and earshot through CE's own adapter trait and
scoring in a scratch crate (outside the workspace; no dependency added).
Result: no third-party detector dominated; all failed on sustained music;
webrtc-vad collapsed at ≤ 5 dB white / 10 dB pink; earshot was most
noise-robust but failed on quiet input and is 16 kHz-only with `unsafe` and
MSRV 1.87. Decision: CE-native `ce.vad.spectral` default, `ce.vad.energy`
fallback, earshot as a recommended future optional adapter.

## 3. Architecture

[audio-intelligence.md](../architecture/audio-intelligence.md),
ADR-0014…0018. New crate `ce-audio-intel` (no new third-party
dependencies, `forbid(unsafe_code)`), contracts in `ce-core::intel`, a
bounded non-blocking **analysis tap** in the Phase 2 capture worker, one
supervised analysis thread per analysed stream, core-owned orchestration
in `runtime/core/intel.rs`, events through the existing bus.

## 4. Modules

| Module | Responsibility |
|---|---|
| `ce-core::intel::config` | `AnalysisConfig` (detector, hop, activity profiles, segment, quality, conditions, adaptive policy, budget incl. `degrade`), `validate()`, `fingerprint()` |
| `ce-core::intel::model` | `VadDescriptor`, `ScoreSemantics`, `AnalysisCapabilities`, `AlgorithmRef`, `ResultMeta`, `DiscontinuityContext`, activity/segment/endpoint/quality/condition/policy/mode types, `AnalysisEvent`, `AnalysisStatus`, `AnalysisCounters` |
| `ce-audio::tap` | `analysis_tap`, `TapSender::offer` (try_send, drop + count), `TapReceiver` (backlog, drops, end reason, revocation discard) |
| `ce-audio-intel::dsp` | radix-2 FFT (tested against a DFT), Hann window, level conversions |
| `…::framer` | blocks → hops + gaps; time-domain and spectral features; per-hop mono export for audio adapters; malformed-input sanitization |
| `…::vad` | `VoiceActivityDetector` trait (`needs_spectral`, `needs_audio`, `score`, `score_audio`), `SpectralVad`, `EnergyVad`, `ScriptedVad`, `FloorTracker` |
| `…::activity` | hysteresis stabilizer (onset/offset thresholds, run lengths, dead band, backdated boundaries) |
| `…::segment` | candidate segments, tiered endpoint candidates, silence windows |
| `…::quality` | windowed quality metrics, usability + reasons (debounced), `ConditionTracker` (6 conditions with hysteresis) |
| `…::policy` | `AdaptivePolicy` (3 reversible decisions), `ResourceGovernor` (Full/Reduced/Shedding with hysteresis) |
| `…::analyzer` | `StreamAnalyzer`: deterministic per-stream pipeline, discontinuity handling, metadata |
| `…::worker` | `spawn_analysis`, `AnalysisHandle` (stop, status, stall detection, abandon), `IntelSignal` |
| `…::synth`, `…::eval` | deterministic signals; WAV reader, scenarios, labels, replay, metrics, digest |
| `communication-engine` | `runtime/core/intel.rs`; facade `start_analysis`, `start_analysis_with`, `stop_analysis`, `analysis_status`; `audio_intel` re-export; prelude additions |

## 5. Data flows, contracts and events

Capture worker → (host queue, unchanged) + (tap, copy) → analysis worker →
`IntelSignal` channel (bounded, 1024) → core → `EventPayload::Analysis` on
the stream's session (topic `Analysis`, `Operational`). Commands
`StartAnalysis` / `StopAnalysis` / `QueryAnalysis` → `CommandOutcome::Analysis`.
Thirteen event kinds (table in audio-intelligence.md); every result carries
`ResultMeta` (stream, sequence range, frame range, time range,
discontinuity, mode). Units and semantics: ADR-0016. Phase 2 contracts were
only extended additively (`MonotonicTime: Default`, `WorkerHandle::
attach_tap/detach_tap`, `RuntimeHealth::analyses` with `serde(default)`,
two metrics); `RuntimeHealth` lost `Eq` (it now holds floats).

## 6. Configuration and capabilities

`EngineConfig::analysis` (defaults) or per-call overrides; full table in
audio-intelligence.md. Validation error code `config.analysis_invalid`.
Capabilities: detector descriptor (id, version, score semantics — the
native detectors report **heuristic confidence, not probability** — rates
8–96 kHz, deterministic, 0 ms look-ahead, 30-hop warm-up), quality on/off,
condition list, `overlap_detection: false`.

## 7. Evaluation methodology, provenance and results

[phase-3-evaluation.md](phase-3-evaluation.md). Ten TTS utterances (CMU
Flite; PROVENANCE.md; bit-exact regeneration verified), 4 dev / 6 test,
energy-derived labels, 8 acoustic conditions, 5 format variants. Weights
tuned on dev only; test reported once frozen. Held-out test split,
`ce.vad.spectral`:

| condition | P | R | F1 | endpoints | false EP |
|---|---|---|---|---|---|
| clean | 0.966 | 1.000 | 0.983 | 6/6 | 0 |
| white 20 dB | 0.987 | 0.963 | 0.975 | 6/6 | 0 |
| white 10 dB | 1.000 | 0.653 | 0.790 | 4/6 | 2 |
| white 5 dB | 1.000 | 0.595 | 0.746 | 2/6 | 1 |
| pink 10 dB | 0.891 | 0.959 | 0.923 | 3/6 | 3 |
| music 10 dB | 0.643 | 1.000 | 0.782 (always on) | 0/6 | 0 |
| clipped +20 dB | 0.898 | 1.000 | 0.946 | 6/6 | 0 |
| quiet −30 dB | 1.000 | 0.931 | 0.964 | 6/6 | 0 |

Quality: clean → Good; low SNR → LowSnr; full-scale clipping → Clipping
(Poor); −30 dB → LowLevel; digital silence → NoSignal (Unusable).
**These are results on synthetic speech and do not predict performance on
real recordings.**

## 8. Performance, stress and resources

Release build, one core: spectral RTF 0.006–0.010, energy ≈ 0.0002; 10 min
of 48 kHz stereo analysed in ≈ 4.2 s with < 400 events/min; peak RSS of the
whole test process 13.1 MiB. Four concurrent live analyses with one
panicking: others unaffected. Overload (load 1.5): tap drops and
Full → Reduced → Shedding, **0 host-queue drops**. Stall detected and
reported. Live mode changes use wall-clock load (not deterministic);
replay is deterministic (bit-for-bit event equality tested).

## 9. Tests and commands

| Gate | Command | Result |
|---|---|---|
| format | `cargo fmt --all -- --check` | pass |
| lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass |
| tests (default) | `cargo test --workspace --all-targets` (+ doctests: 225) | 224 passed, 0 failed |
| tests (all features) | `cargo test --workspace --all-features` | 228 passed, 0 failed |
| replay / regression / stress (release) | `cargo test --release -p ce-audio-intel --test pipeline` | 11 passed |
| evaluation tables | `cargo run --release -p ce-audio-intel --example evaluate -- test` | ran |
| smoke examples | `runtime_smoke`, `audio_smoke`, `analysis_smoke` | PASS |
| docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features` | pass |
| cross-target check | iOS, Android, Windows MSVC, macOS (all crates), wasm32 (core, audio, intel) | pass |
| MSRV | `cargo +1.85.0 check --workspace --all-targets --all-features` | pass |
| dependencies | `cargo deny check` | pass |
| all gates | `scripts/verify.sh` | **all gates passed** (exit 0) |
| Talktual | `tsc --noEmit`, `npm test`, `npm run test:server`, `npm run build` | pass, 537, 60, pass |

New tests: 27 unit tests in `ce-audio-intel`, 2 in `ce-audio::tap`, 11
pipeline tests (replay determinism, test-split regression gate, formats and
hop sizes, discontinuities, format change/restart, quality/conditions/
policy, feature switches, resource policy, hostile input, energy
alternative, 10-minute stress), 9 runtime tests (lifecycle and event order,
errors, revocation, device loss, panic isolation, overload, stall,
shutdown/session close, 4 concurrent live streams), ce-core config/model
tests. The runtime tests were run 25× consecutively without failure after
the races below were fixed.

## 10. Hardware evidence

**None.** No microphone, speaker or audio device exists in this
environment and no user computer was attached. No hardware testing is
claimed.

## 11. Files

New: `crates/ce-audio-intel/` (src: `lib, dsp, framer, vad, activity,
segment, quality, policy, analyzer, worker, synth, eval`; `tests/pipeline.rs`;
`examples/evaluate.rs`), `crates/ce-core/src/intel/{mod,config,model}.rs`,
`crates/ce-audio/src/tap.rs`, `crates/communication-engine/src/runtime/core/
intel.rs`, `…/tests/audio_analysis.rs`, `…/examples/analysis_smoke.rs`,
`fixtures/speech/` (10 WAV, `generate.sh`, `SHA256SUMS`, `PROVENANCE.md`),
ADR-0014…0018, `docs/architecture/audio-intelligence.md`,
`docs/phases/phase-3-{baseline-and-plan,research,evaluation,report}.md`.
Changed: ce-core (`lib`, `command`, `config`, `event`, `health`, `time`),
ce-audio (`lib`, `worker`), communication-engine (`Cargo.toml`, `lib`,
`runtime/core.rs`, `runtime/core/audio.rs`, `runtime/metrics.rs`), workspace
`Cargo.toml`/`Cargo.lock` (internal crate only), `scripts/verify.sh`,
README, ARCHITECTURE, `docs/architecture/{audio,events,privacy}.md`.
Nothing outside `communication-engine/` changed.

## 12. Deviations

* **No real human speech**: evaluation uses synthetic TTS (download of a
  corpus was not authorized). Labels are energy-derived, not human.
* **Neural detectors needing ONNX Runtime** (Silero, TEN) were not
  evaluated (native download not authorized).
* **Overlap detection** is not implemented and is declared unsupported.
* **Shedding** suppresses per-block `Discontinuity` events for the blocks
  it skips (they are counted and reported as lost frames) to keep events
  sparse.
* **CI workflow** unchanged: it already runs `scripts/verify.sh`, which now
  includes the Phase 3 gates.

## 13. Risks

1. Real-speech performance is unknown (TTS is easier than people).
2. Sustained music and other periodic backgrounds are reported as activity.
3. Recall drops at ≤ 10 dB white-noise SNR.
4. Wall-clock resource policy can degrade on a starved host (by design);
   hosts can disable it with `budget.degrade = false`.
5. One thread per analysed stream; many concurrent analyses multiply threads.
6. The scripted benchmark reflects these fixtures only; re-run with real
   recordings before choosing a production detector.

Races found and fixed during testing (recorded for reviewers): a device
lost while the core had not yet marked the stream ended was reported as
`StreamFailed` instead of `StreamInvalidated` (the tap now resolves the
reason from the stream's fault); functional tests on a loaded 4-vCPU
machine legitimately tripped the wall-clock governor (tests now disable
degradation explicitly; resource tests keep it on).

## 14. Acceptance checklist

| Requirement | Status |
|---|---|
| Reconnaissance, research, ADRs | done (§1, §2, ADR-0014…0018) |
| Replaceable VAD adapter, baseline detector, deterministic fake | done (`VoiceActivityDetector`, spectral/energy, `ScriptedVad`; audio-receiving adapters supported) |
| Activity hysteresis | done (`Stabilizer`) |
| Segmentation and endpoint candidates (not turn judgements) | done (`Segmenter`; documented semantics) |
| Silence windows | done |
| Quality analyzer | done (metrics, usability, reasons, `None` when unmeasurable) |
| Limited, non-sensitive condition indicators | done (closed set of 6; ADR-0017) |
| Adaptive and resource policy, degraded modes | done (3 decisions; Full/Reduced/Shedding) |
| Discontinuity handling | done (never bridged; tested) |
| Window/feature pipeline | done (`Framer`) |
| Evaluation and replay with fixture provenance | done (dev/test split, provenance, bit-exact regeneration, deterministic replay) |
| Observability | done (events, status, health, metrics, logs) |
| Orchestrator with supervised workers and bounded queues, never blocks capture | done (tested: 0 host drops under overload) |
| Consume audio only via Phase 2 interfaces | done (additive tap in the capture worker) |
| No ASR/language/diarization/semantics/translation/TTS/cloning/advanced DSP/UI | respected |
| No identity/emotion/health/trait inference | respected |
| All gates incl. Phase 1/2 and Talktual | pass (§9) |
| Hardware testing | **not done — no hardware** |

## 15. Phase 4 readiness

Ready for a consumer of acoustic segments: segments and endpoint candidates
carry exact frame/time ranges and the algorithm fingerprint, so a later
phase can request the corresponding audio and attribute results. Before
relying on accuracy, Phase 4 should (a) evaluate on consented real
recordings, (b) consider the earshot adapter (or an ONNX detector, once a
download is approved) for noisy input, (c) add music/periodic-background
rejection, and (d) define how turn-taking consumes endpoint candidates.
Open items carried over: hardware validation (Phase 2), per-platform
permission integration (Phase 2), the web-vs-native decision (DR-8.5-1).
