# Phase 3 evaluation — methodology, provenance and results

Date: 2026-11-24 · Commit: Phase 3 branch head · Linux x86_64 container
(4 vCPU), Rust 1.97.0, release build. **No audio hardware; no human
recordings.** All numbers below were produced by
`cargo run --release -p ce-audio-intel --example evaluate -- <dev|test|formats>`
and are reproducible bit-for-bit except the real-time factor (RTF).

## Data and provenance

* 10 synthetic English utterances (CMU Flite TTS, 4 voices, 16 kHz mono),
  split 4 dev / 6 test — `fixtures/speech/PROVENANCE.md`, regenerated
  bit-exact by `fixtures/speech/generate.sh` (`SHA256SUMS`).
* Each scenario = 1.2 s lead + utterances separated by 1.0 s + 1.5 s tail
  (dev ≈ 16.2 s, test ≈ 27.9 s), plus a −70 dBFS white floor so silence is never
  digital zero, plus one condition: clean; white noise at 20/10/5 dB SNR;
  pink noise at 10 dB; a sustained chord ("music") at 10 dB; +20 dB gain
  hard-clipped at full scale; −30 dB attenuation. SNR is relative to the RMS
  of reference-active speech. Noise is seeded (deterministic).
* **Reference labels**: per 10 ms frame of the *clean* utterance, active if
  within 40 dB of its peak and above −55 dBFS; gaps < 200 ms bridged;
  regions < 50 ms dropped. These are energy-derived oracle labels, not human
  annotations: they include weak onsets/breaths a human might not mark,
  which inflates onset/offset errors uniformly across detectors.
* **Dev/test discipline**: detector weights (`SpectralWeights::V1`) were
  chosen by a 36-point grid on the dev split only (`evaluate -- tune`,
  objective Σ(F1 − 0.01·FA/min) over all conditions); the test split was run
  after the weights were frozen and is reported unchanged. The regression
  gate (`tests/pipeline.rs::test_split_regression_gate`) uses thresholds
  below these values.

## Metrics

| Metric | Definition |
|---|---|
| P / R / F1 | frame-level (10 ms) precision, recall, F1 of stabilized activity vs reference |
| FA/h | predicted active regions overlapping no reference region (±100 ms), per hour |
| missed ms | reference-active time predicted inactive |
| onset / offset err | mean \|predicted − reference\| boundary, ms, over detected reference regions |
| onset lat | mean decision latency (boundary → decision) of activity onsets, ms |
| frag | predicted regions per detected reference region (1 = ideal) |
| merges | predicted regions covering > 1 reference region |
| endpoints | utterance ends followed by a `SilencePolicy` segment close within 300 ms |
| false EP | `SilencePolicy` closes not within 300 ms of an utterance end |
| EP delay | utterance end → segment-close decision, ms (≈ offset error + 700 ms close policy) |
| usability | last reported quality usability |
| RTF | processing time / audio time, single core |

## Results — test split (held out)

### ce.vad.spectral — Test split

| scenario | P | R | F1 | FA/h | missed ms | onset err | offset err | onset lat | frag | merges | endpoints | false EP | EP delay | usability | RTF |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| clean | 0.966 | 1.000 | 0.983 | 0 | 0 | 364 | 230 | 40 | 1.00 | 1 | 6/6 | 0 | 758 | Some(Good) | 0.0065 |
| white 20 dB | 0.987 | 0.963 | 0.975 | 0 | 660 | 379 | 231 | 60 | 1.00 | 1 | 6/6 | 0 | 627 | Some(Good) | 0.0070 |
| white 10 dB | 1.000 | 0.653 | 0.790 | 0 | 6230 | 188 | 166 | 73 | 1.20 | 0 | 4/6 | 2 | 575 | Some(Fair) | 0.0069 |
| white 5 dB | 1.000 | 0.595 | 0.746 | 0 | 7270 | 43 | 187 | 60 | 1.33 | 0 | 2/6 | 1 | 600 | Some(Fair) | 0.0071 |
| pink 10 dB | 0.891 | 0.959 | 0.923 | 0 | 740 | 379 | 449 | 69 | 1.14 | 1 | 3/6 | 3 | 727 | Some(Fair) | 0.0062 |
| music 10 dB | 0.643 | 1.000 | 0.782 | 0 | 0 | 12284 | 13054 | 40 | 1.00 | 1 | 0/6 | 0 | 0 | Some(Good) | 0.0071 |
| clipped +20 dB | 0.898 | 1.000 | 0.946 | 0 | 0 | 549 | 303 | 40 | 1.00 | 1 | 6/6 | 0 | 827 | Some(Poor) | 0.0061 |
| quiet -30 dB | 1.000 | 0.931 | 0.964 | 0 | 1240 | 29 | 66 | 40 | 1.29 | 0 | 6/6 | 0 | 628 | Some(Fair) | 0.0062 |

### ce.vad.energy — Test split

| scenario | P | R | F1 | FA/h | missed ms | onset err | offset err | onset lat | frag | merges | endpoints | false EP | EP delay | usability | RTF |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| clean | 0.976 | 1.000 | 0.988 | 0 | 0 | 371 | 194 | 40 | 1.00 | 1 | 6/6 | 0 | 718 | Some(Good) | 0.0001 |
| white 20 dB | 0.987 | 0.914 | 0.949 | 0 | 1550 | 376 | 277 | 55 | 1.29 | 1 | 5/6 | 1 | 606 | Some(Good) | 0.0001 |
| white 10 dB | 1.000 | 0.624 | 0.769 | 0 | 6740 | 27 | 197 | 67 | 1.00 | 0 | 2/6 | 1 | 600 | Some(Fair) | 0.0002 |
| white 5 dB | 1.000 | 0.146 | 0.255 | 0 | 15310 | 35 | 2760 | 80 | 1.00 | 0 | 1/6 | 1 | 580 | Some(Fair) | 0.0002 |
| pink 10 dB | 1.000 | 0.625 | 0.769 | 0 | 6720 | 28 | 165 | 64 | 1.25 | 0 | 3/6 | 1 | 597 | Some(Fair) | 0.0001 |
| music 10 dB | 0.779 | 0.838 | 0.807 | 0 | 2910 | 3277 | 5863 | 60 | 1.14 | 1 | 2/6 | 1 | 605 | Some(Fair) | 0.0001 |
| clipped +20 dB | 0.901 | 1.000 | 0.948 | 0 | 0 | 563 | 276 | 40 | 1.00 | 1 | 6/6 | 0 | 803 | Some(Poor) | 0.0001 |
| quiet -30 dB | 0.987 | 0.960 | 0.974 | 0 | 710 | 373 | 241 | 60 | 1.00 | 1 | 6/6 | 0 | 612 | Some(Fair) | 0.0001 |

## Results — development split (used for tuning)

### ce.vad.spectral — Dev split

| scenario | P | R | F1 | FA/h | missed ms | onset err | offset err | onset lat | frag | merges | endpoints | false EP | EP delay | usability | RTF |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| clean | 0.971 | 0.999 | 0.985 | 0 | 10 | 10 | 46 | 40 | 1.00 | 0 | 4/4 | 0 | 758 | Some(Good) | 0.0092 |
| white 20 dB | 0.970 | 0.903 | 0.936 | 0 | 880 | 212 | 524 | 48 | 1.20 | 1 | 4/4 | 0 | 615 | Some(Good) | 0.0094 |
| white 10 dB | 1.000 | 0.784 | 0.879 | 0 | 1960 | 170 | 174 | 60 | 1.20 | 0 | 4/4 | 0 | 530 | Some(Fair) | 0.0095 |
| white 5 dB | 1.000 | 0.375 | 0.546 | 0 | 5680 | 740 | 180 | 67 | 1.00 | 0 | 3/4 | 0 | 520 | Some(Fair) | 0.0100 |
| pink 10 dB | 0.880 | 0.948 | 0.913 | 0 | 470 | 232 | 728 | 60 | 1.00 | 1 | 3/4 | 1 | 753 | Some(Good) | 0.0084 |
| music 10 dB | 0.561 | 1.000 | 0.719 | 0 | 0 | 5504 | 8888 | 40 | 1.00 | 1 | 0/4 | 0 | 0 | Some(Good) | 0.0064 |
| clipped +20 dB | 0.880 | 1.000 | 0.936 | 0 | 0 | 334 | 596 | 40 | 1.00 | 1 | 4/4 | 0 | 808 | Some(Poor) | 0.0102 |
| quiet -30 dB | 1.000 | 0.894 | 0.944 | 0 | 960 | 60 | 76 | 40 | 1.20 | 0 | 4/4 | 0 | 610 | Some(Fair) | 0.0100 |

### ce.vad.energy — Dev split

| scenario | P | R | F1 | FA/h | missed ms | onset err | offset err | onset lat | frag | merges | endpoints | false EP | EP delay | usability | RTF |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| clean | 0.987 | 1.000 | 0.993 | 0 | 0 | 14 | 10 | 40 | 1.00 | 0 | 4/4 | 0 | 712 | Some(Good) | 0.0002 |
| white 20 dB | 0.970 | 0.887 | 0.926 | 0 | 1030 | 224 | 530 | 48 | 1.20 | 1 | 4/4 | 0 | 598 | Some(Good) | 0.0002 |
| white 10 dB | 1.000 | 0.477 | 0.646 | 0 | 4750 | 427 | 183 | 67 | 1.00 | 0 | 3/4 | 0 | 517 | Some(Fair) | 0.0002 |
| white 5 dB | 1.000 | 0.246 | 0.395 | 0 | 6850 | 105 | 660 | 60 | 1.00 | 0 | 1/4 | 1 | 410 | Some(Good) | 0.0002 |
| pink 10 dB | 0.942 | 0.622 | 0.749 | 0 | 3440 | 53 | 237 | 67 | 1.00 | 0 | 2/4 | 1 | 520 | Some(Good) | 0.0001 |
| music 10 dB | 0.808 | 0.749 | 0.777 | 0 | 2280 | 228 | 552 | 56 | 1.00 | 0 | 2/4 | 2 | 525 | Some(Good) | 0.0001 |
| clipped +20 dB | 0.889 | 1.000 | 0.941 | 0 | 0 | 338 | 552 | 40 | 1.00 | 1 | 4/4 | 0 | 778 | Some(Poor) | 0.0001 |
| quiet -30 dB | 1.000 | 0.858 | 0.924 | 0 | 1290 | 56 | 92 | 40 | 1.40 | 0 | 4/4 | 0 | 600 | Some(Fair) | 0.0002 |

## Results — formats and block sizes (test split, white noise 10 dB)

### ce.vad.spectral — Test split

| scenario | P | R | F1 | FA/h | missed ms | onset err | offset err | onset lat | frag | merges | endpoints | false EP | EP delay | usability | RTF |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 16000 Hz x1 / 160 fr | 1.000 | 0.653 | 0.790 | 0 | 6230 | 188 | 166 | 73 | 1.20 | 0 | 4/6 | 2 | 575 | Some(Fair) | 0.0088 |
| 8000 Hz x1 / 80 fr | 1.000 | 0.656 | 0.793 | 0 | 6160 | 194 | 146 | 73 | 1.20 | 0 | 4/6 | 2 | 600 | Some(Fair) | 0.0030 |
| 44100 Hz x1 / 441 fr | 0.984 | 0.767 | 0.862 | 0 | 4180 | 487 | 291 | 69 | 1.14 | 1 | 5/6 | 1 | 552 | Some(Fair) | 0.0067 |
| 48000 Hz x2 / 480 fr | 0.985 | 0.781 | 0.871 | 0 | 3930 | 487 | 291 | 73 | 1.00 | 1 | 5/6 | 1 | 552 | Some(Fair) | 0.0103 |
| 48000 Hz x1 / 1024 fr | 0.985 | 0.781 | 0.871 | 0 | 3930 | 487 | 291 | 73 | 1.00 | 1 | 5/6 | 1 | 552 | Some(Fair) | 0.0096 |

### ce.vad.energy — Test split

| scenario | P | R | F1 | FA/h | missed ms | onset err | offset err | onset lat | frag | merges | endpoints | false EP | EP delay | usability | RTF |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 16000 Hz x1 / 160 fr | 1.000 | 0.624 | 0.769 | 0 | 6740 | 27 | 197 | 67 | 1.00 | 0 | 2/6 | 1 | 600 | Some(Fair) | 0.0002 |
| 8000 Hz x1 / 80 fr | 1.000 | 0.624 | 0.769 | 0 | 6740 | 27 | 197 | 67 | 1.00 | 0 | 2/6 | 1 | 600 | Some(Fair) | 0.0001 |
| 44100 Hz x1 / 441 fr | 1.000 | 0.621 | 0.766 | 0 | 6790 | 35 | 172 | 64 | 1.25 | 0 | 3/6 | 1 | 597 | Some(Fair) | 0.0003 |
| 48000 Hz x2 / 480 fr | 1.000 | 0.632 | 0.775 | 0 | 6590 | 35 | 192 | 70 | 1.00 | 0 | 3/6 | 1 | 570 | Some(Fair) | 0.0005 |
| 48000 Hz x1 / 1024 fr | 1.000 | 0.632 | 0.775 | 0 | 6590 | 35 | 192 | 70 | 1.00 | 0 | 3/6 | 1 | 570 | Some(Fair) | 0.0004 |

The 44.1/48 kHz rows differ from 16 kHz because the test signal is
resampled (linear interpolation) and the analyser decimates by an integer
factor with a boxcar average: the effective spectra differ slightly. 8 kHz
input (no content above 4 kHz) performs like 16 kHz.

## Quality and condition accuracy

From the same runs plus `tests/pipeline.rs`:

| Input | Expected | Reported |
|---|---|---|
| clean speech | Good, no reasons | Good (test and dev) |
| white 10 / 5 dB | LowSnr | Fair with LowSnr (5 dB SNR *estimate* is ≈ 6–7 dB because the active level includes noise, so not Poor) |
| clipped +20 dB at full scale | Clipping | Poor with Clipping (SuppressLevelEstimates is exercised by the tone test below) |
| −30 dB speech | LowLevel | Fair with LowLevel |
| digital silence | NoSignal | Unusable; LowConfidenceActivity applied |
| loud clipped tone, then silence, then noise | Clipping, Overload, SustainedSilence, SustainedNoise | all raised and cleared with hysteresis (`quality_conditions_and_adaptive_policy_respond_to_the_signal`) |

## Performance, stress and resources

| Measurement | Result | Source |
|---|---|---|
| RTF, spectral, 16 kHz mono | 0.006–0.010 (≈ 100–160× real time) | evaluate (release) |
| RTF, energy | 0.0002–0.0006 | evaluate (release) |
| 10 min, 48 kHz stereo, 6 000 blocks | 4.1–4.3 s (RTF ≈ 0.007), < 400 events/min | `long_streams_stay_bounded_and_fast` (release) |
| Peak RSS of the whole test process during that run | 13.1 MiB | `getrusage` (release) |
| 4 concurrent live streams, one detector panicking | others unaffected; all streams kept delivering | `concurrent_analyses_on_live_streams_are_isolated` |
| Overload (15 ms work per 10 ms block) | tap drops + Full → Reduced → Shedding; host queue dropped 0 blocks | `overload_degrades_and_drops_analysis_copies_but_never_capture` |
| Stall (600 ms hop) | `analysis.stalled` within ≈ 150 ms + housekeeping tick | `a_stalled_worker_is_detected_and_reported` |

Per-stream state is fixed after the first block (FFT tables and buffers
sized by format): framer history ≤ 40 ms of 16 kHz mono, a 1024-point
complex FFT, ≤ 3 s of hop levels for the noise floor, 0.5 s for modulation.

## Limitations

1. Synthetic speech only; no human recordings, reverberation, far-field,
   overlapping talkers, or real devices.
2. Labels are energy-derived, not human.
3. Small set (30 s test audio, 6 utterances): differences of a few
   hundredths in F1 are not significant.
4. Sustained music defeats every detector tested.
5. Live resource decisions use wall-clock load and are not deterministic.
