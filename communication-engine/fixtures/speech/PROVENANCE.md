# Speech fixtures — provenance

Ten short English utterances used by the Phase 3 evaluation and replay
tests. **They are synthetic (text-to-speech), not recordings of people.**
No person's voice, identity or personal data is contained in them.

| Property | Value |
|---|---|
| Synthesizer | CMU Flite, via FFmpeg's `flite` lavfi source (FFmpeg 6.1.1-3ubuntu5, Ubuntu `ffmpeg` package) |
| Voices | `kal16`, `slt`, `rms`, `awb` (the CMU voices bundled with Flite) |
| Format | RIFF WAV, PCM s16le, 16 000 Hz, mono, no metadata (`-bitexact -map_metadata -1`) |
| Size | ≈ 1 MB total, 0.7–6.5 s each |
| Generator | [`generate.sh`](generate.sh) — the exact commands, voices, texts and tempo |
| Integrity | [`SHA256SUMS`](SHA256SUMS); regeneration was verified **bit-exact** on 2026-11-24 |
| License | CMU Flite and its voices are distributed under the CMU/Flite BSD-style permissive license (redistribution and modification permitted with the notice retained). Generated audio carries no additional restriction. |

espeak-ng was available but **not** used: its voice data is GPL-3, which
makes the licensing of generated audio ambiguous.

## Splits

* **Development** (`dev_*`, 4 files, 10.5 s): used to choose detector
  weights (`SpectralWeights::V1`) and thresholds.
* **Test** (`test_*`, 6 files, 20.2 s): held out; only reported. The
  regression gate (`ce-audio-intel/tests/pipeline.rs`) checks it against
  thresholds set *below* the reported values.

| File | Voice | Tempo | Text |
|---|---|---|---|
| dev_kal16_station | kal16 | 1.0 | Good morning, could you tell me where the train station is |
| dev_slt_coffee | slt | 1.0 | I would like a cup of coffee and a glass of water, please |
| dev_rms_meeting | rms | 1.0 | The meeting starts at nine thirty tomorrow |
| dev_awb_yes | awb | 1.0 | Yes |
| test_slt_thanks | slt | 1.0 | Thank you very much for your help today |
| test_kal16_slowly | kal16 | 1.0 | Can you please speak a little more slowly, I am still learning |
| test_awb_directions | awb | 1.0 | Turn left at the second corner and walk straight for two blocks |
| test_rms_no | rms | 1.0 | No |
| test_slt_long | slt | 1.0 | We should leave early, because the traffic will be heavy in the evening, and the road near the bridge is closed for repairs |
| test_kal16_fast | kal16 | 1.5 | How much does this cost, and do you accept cards |

## Derived material (generated in code, not stored)

Noise (white, pink), "music" (a sustained chord), clipping, level changes,
resampling (8–96 kHz) and channel duplication are applied by
`ce_audio_intel::eval::build` with fixed seeds; reference activity labels
are derived from the clean utterances by energy trimming (see
`ce_audio_intel::eval`). Both are deterministic.

## Limitations

TTS speech is cleaner and more regular than human speech (no breath noise,
hesitations, overlapping talkers, reverberation or microphone coloration).
Results on these fixtures are **not** evidence of performance on real
recordings. A real-speech set (e.g. LibriSpeech, CC BY 4.0) was not
downloaded: large downloads were not authorized for this phase.
