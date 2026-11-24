#!/usr/bin/env bash
# Run Talktual end to end with the Communication Engine recognizer, for a
# human test on this computer:
#   1. the Talktual <-> CE bridge (real sherpa-onnx model) on ws://127.0.0.1:8091
#   2. the Talktual signaling server on http://localhost:8080
#   3. the Talktual web app (Vite dev) on http://localhost:5173, built with
#      VITE_ASR_ENGINE=ce
# Needs: Rust (rustup), Node 20+, and the model (adapters/ce-asr-sherpa/scripts/fetch-model.sh).
# Usage: CE_SHERPA_MODEL_DIR=~/ce-models/sherpa-onnx-streaming-zipformer-en-2023-06-26 \
#        communication-engine/adapters/talktual-bridge/scripts/run-talktual-ce.sh
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$here/../../.." && pwd)"
: "${CE_SHERPA_MODEL_DIR:?set CE_SHERPA_MODEL_DIR (run adapters/ce-asr-sherpa/scripts/fetch-model.sh first)}"
if [[ -d "${SHERPA_ONNX_ARCHIVE_DIR:-/nonexistent}" ]]; then export SHERPA_ONNX_ARCHIVE_DIR; fi

echo "==> building the bridge (first build downloads/links sherpa-onnx; takes a few minutes)"
(cd "$here" && cargo build --release)

pids=()
cleanup() { for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT INT TERM

echo "==> starting the bridge"
"$here/target/release/talktual-bridge" --listen 127.0.0.1:8091 &
pids+=($!)

echo "==> starting the signaling server"
(cd "$repo/signaling-server" && { [[ -d node_modules ]] || npm ci; } && npm start) &
pids+=($!)

echo "==> starting Talktual (VITE_ASR_ENGINE=ce)"
cd "$repo"
[[ -d node_modules ]] || npm ci
VITE_ASR_ENGINE=ce VITE_CE_BRIDGE_URL=ws://127.0.0.1:8091 VITE_SIGNALING_URL=ws://localhost:8080 \
  npx vite --host localhost --port 5173 --strictPort &
pids+=($!)

echo
echo "Open http://localhost:5173 in two browser windows (Chrome, Edge, Firefox or Safari),"
echo "start a call in one and join it from the other. Speak English while holding"
echo "Push to talk in the first window. Ctrl+C here stops everything."
wait
