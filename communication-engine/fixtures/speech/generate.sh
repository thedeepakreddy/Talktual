#!/usr/bin/env bash
# Regenerates the Phase 3 speech fixtures (see PROVENANCE.md).
# Requires ffmpeg built with libflite (Debian/Ubuntu: apt-get install ffmpeg).
# Output is bit-exact: compare with `sha256sum -c SHA256SUMS`.
# Usage: fixtures/speech/generate.sh [output-dir]   (default: this directory)
set -euo pipefail
out="${1:-$(dirname "$0")}"
mkdir -p "$out"
gen() {
  local name=$1 voice=$2 tempo=$3 text=$4
  ffmpeg -hide_banner -loglevel error -y -f lavfi -i "flite=voice=$voice:text='$text'" \
    -af "aresample=16000,atempo=$tempo" -ac 1 -ar 16000 -sample_fmt s16 \
    -bitexact -map_metadata -1 -fflags +bitexact "$out/$name.wav"
}
# Development split (used to set parameters).
gen dev_kal16_station kal16 1.0 "Good morning, could you tell me where the train station is"
gen dev_slt_coffee slt 1.0 "I would like a cup of coffee and a glass of water, please"
gen dev_rms_meeting rms 1.0 "The meeting starts at nine thirty tomorrow"
gen dev_awb_yes awb 1.0 "Yes"
# Test split (held out; only reported).
gen test_slt_thanks slt 1.0 "Thank you very much for your help today"
gen test_kal16_slowly kal16 1.0 "Can you please speak a little more slowly, I am still learning"
gen test_awb_directions awb 1.0 "Turn left at the second corner and walk straight for two blocks"
gen test_rms_no rms 1.0 "No"
gen test_slt_long slt 1.0 "We should leave early, because the traffic will be heavy in the evening, and the road near the bridge is closed for repairs"
gen test_kal16_fast kal16 1.5 "How much does this cost, and do you accept cards"
