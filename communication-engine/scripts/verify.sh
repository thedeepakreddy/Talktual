#!/usr/bin/env bash
# Phase 1 verification gates for the Communication Engine workspace.
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

step "tests (all features, including doctests)"
cargo test --workspace --all-features

step "documentation (warnings denied)"
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features

step "smoke example"
cargo run -p communication-engine --example runtime_smoke

step "cross-target check (targets installed via rustup only)"
for target in aarch64-apple-ios aarch64-linux-android x86_64-pc-windows-msvc aarch64-apple-darwin; do
  if rustup target list --installed 2>/dev/null | grep -qx "$target"; then
    cargo check -p ce-core -p communication-engine --all-features --target "$target"
  else
    echo "skip $target (not installed: rustup target add $target)"
  fi
done
if rustup target list --installed 2>/dev/null | grep -qx wasm32-unknown-unknown; then
  cargo check -p ce-core --target wasm32-unknown-unknown
fi

if [[ "${CE_SKIP_DENY:-0}" != "1" ]]; then
  step "dependency licenses, bans, sources and advisories"
  cargo deny check
fi

step "all gates passed"
