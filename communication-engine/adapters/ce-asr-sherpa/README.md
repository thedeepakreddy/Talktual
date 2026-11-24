# ce-asr-sherpa — optional real ASR adapter (Phase 4 baseline)

Streaming English speech recognition for the Communication Engine with the
**sherpa-onnx streaming Zipformer** (INT8). ADR:
[0023](../../docs/architecture/adr/0023-real-asr-baseline-sherpa.md).
Results: [phase-4-real-asr.md](../../docs/phases/phase-4-real-asr.md).

**Not part of the CE workspace.** Its build links native sherpa-onnx and
ONNX Runtime libraries and needs Rust ≥ 1.89 (CE: 1.85). It is built and
verified separately; CE's own gates never build it.

## Provision (≈ 334 MB downloaded: 310 MB model archive + 24 MB native libraries; ≈ 95 MB kept)

```bash
scripts/fetch-model.sh ~/ce-models        # model INT8 files + native libs, checksum-verified
export CE_SHERPA_MODEL_DIR=~/ce-models/sherpa-onnx-streaming-zipformer-en-2023-06-26
export SHERPA_ONNX_ARCHIVE_DIR=~/ce-models/native
```

The model archive (310 MB) also contains FP32 weights and sample
recordings. The script extracts **only** the INT8 transducer files,
`tokens.txt` and `README.md`, then deletes the archive.

## Use

```rust
use ce_asr_sherpa::{SherpaConfig, SherpaFactory};
let asr = SherpaFactory::new(SherpaConfig::zipformer_en_2023_06_26_int8(model_dir));
let engine = communication_engine::Engine::builder()
    .audio_backend(backend)
    .asr_adapter(asr)          // default adapter; nothing loads until a recognition starts
    .build()?;
```

## Verify

```bash
scripts/verify.sh                                   # fmt, clippy, tests (incl. real-model tests), cargo deny, resources
cargo run --release --example evaluate_real -- all  # full real-model evaluation tables
```

Platforms: verified on Linux x86_64 only. sherpa-onnx ships native
libraries for macOS, Windows, Linux aarch64, Android and iOS; none of these
were built here.

## Licenses

* Adapter dependencies: `sherpa-onnx` and `sherpa-onnx-sys` (Apache-2.0),
  `serde` and `serde_json`.
* Build-only dependencies of `sherpa-onnx-sys` use permissive licenses
  (BSD, ISC, Zlib, bzip2, CC0/MIT-0, CDLA-Permissive-2.0); see
  `deny.toml`.
* Linked native code: sherpa-onnx (Apache-2.0), ONNX Runtime (MIT), kaldi
  components (Apache-2.0), sentencepiece (Apache-2.0), kissfft (BSD-3),
  utf8proc (MIT), OpenFst (Apache-2.0).
* Model: Apache-2.0.
