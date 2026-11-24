#!/usr/bin/env bash
# Gates for the optional sherpa-onnx adapter (outside the CE workspace).
# Needs: scripts/fetch-model.sh run first; CE_SHERPA_MODEL_DIR and
# SHERPA_ONNX_ARCHIVE_DIR exported. Linux x86_64 only (verified there).
set -euo pipefail
cd "$(dirname "$0")/.."
: "${CE_SHERPA_MODEL_DIR:?run scripts/fetch-model.sh and export CE_SHERPA_MODEL_DIR}"
: "${SHERPA_ONNX_ARCHIVE_DIR:?export SHERPA_ONNX_ARCHIVE_DIR (see scripts/fetch-model.sh)}"
step() { printf '\n==> %s\n' "$*"; }
step "format";  cargo fmt -- --check
step "clippy";  cargo clippy --all-targets -- -D warnings
step "tests (no model needed)"; cargo test --release
step "tests (real model)"; cargo test --release -- --ignored
step "dependency licenses, bans, sources and advisories"
cargo deny check   # policy: ./deny.toml (CE policy + build-only permissive licenses)
step "real-model evaluation (resources section)"
cargo run --release --example evaluate_real -- resources
step "adapter gates passed"
