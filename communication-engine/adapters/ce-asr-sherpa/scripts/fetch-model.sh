#!/usr/bin/env bash
# Provision the approved sherpa-onnx baseline (Phase 4, ADR-0023):
#   * the streaming Zipformer English model — only its INT8 transducer files,
#     tokens and README are extracted (the archive also contains FP32
#     weights and sample recordings, which are NOT extracted);
#   * the sherpa-onnx v2.0.3 static libraries for linux-x64 (normally
#     downloaded by sherpa-onnx-sys's build script; fetched here with curl
#     so the system CA store and proxy settings apply).
# Checksums were recorded when first downloaded (2026-11-24); a mismatch
# aborts. Usage: scripts/fetch-model.sh [dest]   (default: ~/ce-models)
# Then: export CE_SHERPA_MODEL_DIR=<dest>/sherpa-onnx-streaming-zipformer-en-2023-06-26
#       export SHERPA_ONNX_ARCHIVE_DIR=<dest>/native
set -euo pipefail
command -v sha256sum >/dev/null || sha256sum() { shasum -a 256 "$@"; }   # macOS
dest="${1:-$HOME/ce-models}"
base=https://github.com/k2-fsa/sherpa-onnx/releases/download
m=sherpa-onnx-streaming-zipformer-en-2023-06-26
mkdir -p "$dest/native"
cd "$dest"
if [[ ! -f "$m/SHA256SUMS" ]]; then
  curl -fSL -o model.tar.bz2 "$base/asr-models/$m.tar.bz2"
  echo "639e25b578e9e997131402199419c13a941f8e4e198e2da1ce57dbf5cf401282  model.tar.bz2" | sha256sum -c -
  tar -xjf model.tar.bz2 \
    "$m/encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx" \
    "$m/decoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx" \
    "$m/joiner-epoch-99-avg-1-chunk-16-left-128.int8.onnx" \
    "$m/tokens.txt" "$m/README.md"
  rm model.tar.bz2
  cat > "$m/SHA256SUMS" <<'SUMS'
780c63ee94c7cfa314211172e5d09b406c0da2beab5c40ea2f54cc95670b76a5  decoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx
5022b2eca5b19d1bc104fcf33e26bc32604b7df553cd2e1f62e31dc7b05e9c87  encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx
abd5e30f3f16fc510605c6029dba33f10e4386bd75c5bdc30cf94076864db10d  joiner-epoch-99-avg-1-chunk-16-left-128.int8.onnx
49e3c2646595fd907228b3c6787069658f67b17377c60aeb8619c4551b2316fb  tokens.txt
SUMS
fi
(cd "$m" && sha256sum -c SHA256SUMS)
if [[ "$(uname -sm)" == "Linux x86_64" ]]; then
  lib=sherpa-onnx-v2.0.3-linux-x64-static-lib.tar.bz2
  if [[ ! -f "native/$lib" ]]; then
    curl -fSL -o "native/$lib" "$base/v2.0.3/$lib"
  fi
  echo "f927e695d29b8c7ea3fda97eed215204b210fd57f9faf990a93377bac098ad57  native/$lib" | sha256sum -c -
else
  # Other platforms: sherpa-onnx-sys's build script downloads the matching
  # prebuilt library itself (not verified by CE on these platforms).
  echo "note: native libraries for $(uname -sm) will be downloaded by the build; SHERPA_ONNX_ARCHIVE_DIR is not needed"
fi
echo "CE_SHERPA_MODEL_DIR=$dest/$m"
[[ -n "$(ls "$dest/native" 2>/dev/null)" ]] && echo "SHERPA_ONNX_ARCHIVE_DIR=$dest/native"
exit 0
