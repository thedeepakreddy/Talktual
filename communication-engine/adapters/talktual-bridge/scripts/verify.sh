#!/usr/bin/env bash
# Gates for the Talktual <-> CE bridge (outside the CE workspace).
# Needs CE_SHERPA_MODEL_DIR (and SHERPA_ONNX_ARCHIVE_DIR on Linux x86_64 behind a proxy).
set -euo pipefail
cd "$(dirname "$0")/.."
: "${CE_SHERPA_MODEL_DIR:?run ../ce-asr-sherpa/scripts/fetch-model.sh and export CE_SHERPA_MODEL_DIR}"
step() { printf '\n==> %s\n' "$*"; }
step "format";  cargo fmt -- --check
step "clippy";  cargo clippy --all-targets -- -D warnings
step "tests (protocol, scripted adapter)"; cargo test --release
step "tests (real model over the WebSocket)"; cargo test --release -- --ignored
step "dependency licenses, bans, sources and advisories"; cargo deny check
step "bridge gates passed"
