### Overall (all language pairs)

18 speakers × 30 ordered pairs; descriptor coverage 100%; different-speaker cosine (reference) 0.765

| Condition | n | Cosine mean | Top-1 ident. | Mean rank | Pitch err median (st) | Pitch err p90 | Tempo err median (%) | Tempo err p90 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `baseline` | 540 | 0.741 | 5.6% | 9.5 | 6.70 | 17.12 | 14.6 | 25.0 |
| `vm_prosody` | 540 | 0.800 | 13.7% | 6.5 | 1.10 | 8.40 | 14.6 | 25.0 |
| `vm_register` | 540 | 0.775 | 6.1% | 7.8 | 5.00 | 10.10 | 14.6 | 25.0 |
| `oracle_prosody` | 540 | 0.800 | 15.2% | 6.4 | 0.60 | 8.41 | 0.0 | 0.0 |
| `oracle_select` | 540 | 0.826 | 2.8% | 4.6 | 0.40 | 1.00 | 0.0 | 0.0 |
| `oracle_full` | 540 | 0.898 | 85.7% | 1.2 | 0.50 | 1.10 | 0.0 | 0.0 |

### Per target language (mean over source languages)

| Target | baseline cos / pitch-err st | vm_prosody cos / pitch-err st | vm_register cos / pitch-err st | oracle_full cos / pitch-err st |
| --- | --- | --- | --- | --- |
| de | 0.736 / 7.37 | 0.797 / 1.11 | 0.782 / 5.74 | 0.911 / 0.80 |
| en | 0.704 / 6.43 | 0.762 / 1.05 | 0.758 / 4.77 | 0.863 / 0.36 |
| es | 0.738 / 6.40 | 0.795 / 1.05 | 0.757 / 4.89 | 0.902 / 0.40 |
| hi | 0.752 / 6.22 | 0.820 / 0.96 | 0.779 / 4.74 | 0.919 / 0.35 |
| ja | 0.747 / 6.94 | 0.802 / 0.93 | 0.776 / 5.05 | 0.881 / 0.35 |
| ru | 0.767 / 6.02 | 0.823 / 1.16 | 0.799 / 4.58 | 0.913 / 0.75 |

### Per pair: vm_prosody − baseline (cosine Δ, pitch-error Δ st)

| Pair | Δ cosine | Δ pitch err | vm_prosody top-1 |
| --- | --- | --- | --- |
| de->en | +0.056 | -5.20 | 11% |
| de->es | +0.056 | -5.30 | 17% |
| de->hi | +0.053 | -4.95 | 17% |
| de->ja | +0.044 | -5.50 | 11% |
| de->ru | +0.053 | -4.75 | 17% |
| en->de | +0.072 | -6.65 | 17% |
| en->es | +0.070 | -6.05 | 17% |
| en->hi | +0.071 | -5.80 | 17% |
| en->ja | +0.058 | -6.60 | 11% |
| en->ru | +0.061 | -5.30 | 11% |
| es->de | +0.075 | -6.60 | 11% |
| es->en | +0.065 | -6.10 | 11% |
| es->hi | +0.085 | -5.90 | 17% |
| es->ja | +0.063 | -6.40 | 17% |
| es->ru | +0.058 | -5.55 | 17% |
| hi->de | +0.060 | -5.43 | 17% |
| hi->en | +0.071 | -5.20 | 11% |
| hi->es | +0.059 | -5.13 | 17% |
| hi->ja | +0.057 | -5.53 | 11% |
| hi->ru | +0.059 | -4.50 | 11% |
| ja->de | +0.049 | -6.30 | 6% |
| ja->en | +0.050 | -5.05 | 11% |
| ja->es | +0.048 | -4.90 | 17% |
| ja->hi | +0.066 | -4.60 | 11% |
| ja->ru | +0.047 | -4.15 | 11% |
| ru->de | +0.053 | -6.30 | 11% |
| ru->en | +0.049 | -5.30 | 11% |
| ru->es | +0.050 | -5.35 | 11% |
| ru->hi | +0.065 | -5.05 | 22% |
| ru->ja | +0.053 | -6.00 | 17% |

### vm_prosody tempo error by true speaker tempo

| True speed (wpm) | n | median tempo err (%) |
| --- | --- | --- |
| 140 | 180 | 25.0 |
| 175 | 180 | 0.0 |
| 205 | 180 | 14.6 |

### Calibration

- reference syllable rate at 175 wpm: 5.94 (product constant 5.9; matches: True)
- tempo rank correlation (English, calibration speakers): 0.879
- voiced seconds per single sentence: median 1.25

### Analyzer accuracy (exact synthetic tones)

- pitch abs error: median 0.069 st, max 0.142 st (n=12, 80–350 Hz, 16/48 kHz)

### Intelligibility proxy (pocketsphinx WER, English targets; relative only)

| Condition | n | WER mean | WER median |
| --- | --- | --- | --- |
| `baseline` | 80 | 0.66 | 0.66 |
| `vm_prosody` | 80 | 0.75 | 0.86 |
| `vm_register` | 80 | 0.90 | 0.86 |
| `oracle_prosody` | 80 | 0.77 | 0.86 |
| `oracle_select` | 80 | 0.76 | 0.75 |
| `oracle_full` | 80 | 0.76 | 0.75 |

| Parameter extreme (default voice) | WER mean |
| --- | --- |
| pitch_low_clamp | 0.68 |
| pitch_high_clamp | 0.80 |
| tempo_0.80 | 0.70 |
| tempo_1.25 | 0.84 |
| neutral | 0.73 |

### Robustness (analyzer → descriptor)

| Input | Descriptor produced | Same pitch step as clean | Pitch drift vs clean (median st) |
| --- | --- | --- | --- |
| noise_20 | 100% | 89% | 0.00 |
| noise_10 | 100% | 83% | 0.13 |
| noise_0 | 0% | — | — |
| short_0.5 | 0% | — | — |
| short_1.0 | 0% | — | — |
| short_2.0 | 56% | 60% | 0.25 |
| clipped | 100% | 89% | 0.00 |
| quiet | 100% | 78% | 0.25 |
| resample_48k | 100% | 100% | 0.12 |
| silence | 0% | — | — |
| noise_only | 0% | — | — |
| clean | 100% | — | — |

### Resources

- analyzer: 2.18 ms per audio second (Node v22.23.5, 4 vCPU, no GPU; 48 s at 22050 Hz)
- descriptor: 58 bytes JSON

Run: 2026-11-21T15:56:48, 25.1 min, eSpeak NG text-to-speech: 1.51  Data at: /usr/lib/x86_64-linux-gnu/espeak-ng-data
