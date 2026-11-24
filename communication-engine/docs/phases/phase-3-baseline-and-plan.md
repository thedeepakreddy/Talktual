# Phase 3 baseline and plan — Audio intelligence

Date: 2026-11-24 · Branch: `deepak/gallant-hamilton-wegi2i` · Base: Phase 2
commit `fd18b68` (not yet merged to `main`).

## Baseline (measured before any Phase 3 change was relied on)

The Phase 2 commit was checked out in a separate worktree and its suite run
unchanged:

| Gate | Result |
|---|---|
| `cargo test --workspace` | 172 passed, 0 failed |
| `cargo test --workspace --all-features` | 175 passed, 0 failed |
| Talktual `tsc --noEmit` | pass |
| Talktual `npm test` | 537 passed (30 files) |
| Talktual `npm run test:server` | 60 passed |
| Talktual `npm run build` | pass |

(Talktual gates were run on the Phase 3 tree; Phase 3 changes nothing
outside `communication-engine/`, verified with `git status`.)

## Environment findings

* No audio hardware (`/dev/snd` absent, no attached computer): all audio
  comes from the test kit's fake backend and from fixtures.
* No human speech corpus is available offline; downloading one (e.g.
  LibriSpeech) was not authorized. Fixtures are therefore synthetic TTS
  (CMU Flite through FFmpeg), with provenance and bit-exact regeneration.
* ONNX Runtime is not available and downloading it was not authorized, so
  neural detectors that need it (Silero, TEN) could not be evaluated.

## Interfaces consumed (Phase 2, unchanged contracts)

`AudioBlock { meta: AudioFrameMeta, samples }` (interleaved f32 at the
device rate, sample-clock `timestamp`, `frame_index`, `sequence`,
`discontinuity`), the capture worker (one new, additive hook: the analysis
tap), stream lifecycle and authorization in the runtime core.

## Plan (as executed)

1. Contracts in `ce-core::intel` (config + validation + fingerprint, results,
   events, status, capabilities); command/event/health additions.
2. `ce-audio::tap` — bounded non-blocking copy from the capture worker.
3. `ce-audio-intel` — framer/features, detectors (spectral, energy,
   scripted), stabilizer, segmenter, quality, conditions, policy, governor,
   analyzer, supervised worker, synthetic signals, evaluation/replay.
4. Runtime integration (`runtime/core/intel.rs`) and facade methods.
5. Fixtures + provenance; scratch benchmark of third-party detectors.
6. Tune on dev, freeze, report test; regression gate below reported values.
7. Unit, integration, replay, fault, stress tests; smoke example.
8. ADR-0014…0018, architecture doc, research/evaluation/report; verify.sh.
