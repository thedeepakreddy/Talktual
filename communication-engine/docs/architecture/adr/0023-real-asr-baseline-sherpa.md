# ADR-0023 — Real-ASR baseline: sherpa-onnx streaming Zipformer (optional adapter)

Status: Accepted (Phase 4) · Date: 2026-11-24 · Approval: the user approved
downloading and integrating the sherpa-onnx streaming Zipformer English
INT8 model. Real speech corpora remain **unapproved**.

## Decision
* **Model**: `sherpa-onnx-streaming-zipformer-en-2023-06-26` (Apache-2.0,
  trained on LibriSpeech per its README). Only the INT8 encoder, decoder
  and joiner (70.9 MB) plus `tokens.txt` are used. The release archive's
  FP32 weights and sample recordings are **not extracted**, and the archive
  is deleted after extraction. Checksums are recorded in
  `adapters/ce-asr-sherpa/scripts/fetch-model.sh`. The model is never
  committed.
* **Binding**: crates `sherpa-onnx` / `sherpa-onnx-sys` 2.0.3 (Apache-2.0),
  which statically link the prebuilt sherpa-onnx C API and ONNX Runtime
  (24 MB archive for linux-x64, fetched by the provisioning script with
  curl and passed via `SHERPA_ONNX_ARCHIVE_DIR`; the build script's own
  downloader does not use the system CA store).
* **Placement**: a separate crate, `adapters/ce-asr-sherpa`, **outside the
  CE workspace** (`exclude`), with its own lockfile, `deny.toml` and
  `scripts/verify.sh`. Reasons:
  1. the binding's build dependencies require Rust ≥ 1.89, while CE's
     MSRV is 1.85;
  2. its build downloads or links native libraries per target, which
     would break `--workspace --all-features`, the cross-target checks and
     CI;
  3. CE's own crates gain no dependency. The default build is unchanged.

  An optional Cargo feature inside `ce-speech` was rejected: the workspace
  gates enable all features.
* **Adapter** (`ce.asr.sherpa-zipformer`) implements the unchanged
  `AsrFactory` / `AsrAdapter` traits:
  * discovery reads only file sizes and the optional `SHA256SUMS`;
  * `create` validates that the files exist;
  * `load` runs on the recognition worker and creates the ONNX sessions
    once per factory, shared by every adapter while any holds them;
  * each utterance gets its own sherpa-onnx stream;
  * greedy search, 1 thread, and the sherpa-onnx endpoint rules
    (2.4 s / 1.2 s / 20 s);
  * 300 ms of zero tail padding before `input_finished`, per sherpa-onnx
    practice.

  Declared capabilities: streaming, partials (may revise), endpointing,
  English, 16 kHz, deterministic with 1 thread. Not declared: punctuation,
  casing, word timestamps (token start times only), scores and language
  reports. Non-English hints are refused (`asr.language_unsupported`).
* **Pipeline change found with the real model**: Phase 3 onsets fire after
  soft word beginnings, so utterances now start `lead_in_ms` (default 500)
  before the onset, served from a pre-roll ring whose default grew from
  500 ms to 1000 ms. Both values were chosen on the dev split with the
  real model. The frame range includes the lead-in; nothing is shifted.
* **Commit policy**: unchanged after a dev comparison with the real model
  (identical transcripts across policies; 0 amendments with the default).

## Consequences
CE now recognizes English speech when the optional adapter is built and
the model is provisioned. All results so far are on synthetic TTS speech
(see `docs/phases/phase-4-real-asr.md`). Each further platform needs its
native sherpa-onnx libraries and its own verification. The adapter's
gates run separately from the workspace's (`adapters/ce-asr-sherpa/scripts/verify.sh`).
