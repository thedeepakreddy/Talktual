#!/usr/bin/env bash
# Verification gates for the Communication Engine workspace (Phases 1-5).
# Usage: scripts/verify.sh            (from anywhere)
#        CE_SKIP_DENY=1 scripts/verify.sh   (skip cargo-deny if not installed)
set -euo pipefail
cd "$(dirname "$0")/.."

step() { printf '\n==> %s\n' "$*"; }

step "rustc / cargo"
rustc --version && cargo --version

step "format"
cargo fmt --all -- --check

step "clippy (all targets, all features, warnings denied)"
cargo clippy --workspace --all-targets --all-features -- -D warnings

step "tests (all targets, default features)"
cargo test --workspace --all-targets

step "tests (all features, including doctests and the cpal adapter on ALSA's null device)"
# On Linux the cpal feature needs the ALSA development package (libasound2-dev).
cargo test --workspace --all-features

step "documentation (warnings denied)"
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features

step "smoke examples"
cargo run -p communication-engine --example runtime_smoke
cargo run -p communication-engine --example audio_smoke
cargo run -p communication-engine --example analysis_smoke
cargo run -p communication-engine --example speech_smoke

step "audio intelligence: optimized replay, regression gate and stress"
cargo test --release -p ce-audio-intel --test pipeline
cargo run --release -p ce-audio-intel --example evaluate -- test

step "speech: optimized replay, held-out regression gate and evaluation tables"
cargo test --release -p ce-speech --test evaluation
cargo run --release -p ce-speech --example evaluate_speech -- robustness

step "semantics: replay regression gate (pinned scores) and evaluation report"
cargo test --release -p ce-semantic --test evaluation
cargo run --release -p ce-semantic --example evaluate_semantic -- all

step "cross-target check (compile only; targets installed via rustup only)"
for target in aarch64-apple-ios aarch64-linux-android x86_64-pc-windows-msvc aarch64-apple-darwin; do
  if rustup target list --installed 2>/dev/null | grep -qx "$target"; then
    cargo check -p ce-core -p ce-audio -p ce-audio-intel -p ce-speech -p ce-semantic -p communication-engine --all-features --target "$target"
  else
    echo "skip $target (not installed: rustup target add $target)"
  fi
done
if rustup target list --installed 2>/dev/null | grep -qx wasm32-unknown-unknown; then
  cargo check -p ce-core -p ce-audio -p ce-audio-intel -p ce-speech -p ce-semantic --target wasm32-unknown-unknown
fi

if [[ "${CE_SKIP_DENY:-0}" != "1" ]]; then
  step "dependency licenses, bans, sources and advisories"
  cargo deny check
fi

step "all gates passed"
