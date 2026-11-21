### Overall (all language pairs)

18 speakers × 30 ordered pairs; descriptor coverage 100%; different-speaker cosine (reference) 0.765

| Condition | n | Cosine mean | Top-1 ident. | Mean rank | Pitch err median (st) | Pitch err p90 | Tempo err median (%) | Tempo err p90 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `baseline` | 540 | 0.741 | 5.6% | 9.5 | 6.70 | 17.12 | 14.6 | 25.0 |
| `vm_prosody` | 540 | 0.803 | 14.1% | 6.4 | 1.10 | 8.40 | 15.0 | 37.5 |
| `vm_register` | 540 | 0.773 | 5.7% | 7.8 | 5.00 | 10.10 | 15.0 | 37.5 |
| `oracle_prosody` | 540 | 0.800 | 15.2% | 6.4 | 0.60 | 8.41 | 0.0 | 0.0 |
| `oracle_select` | 540 | 0.826 | 2.8% | 4.6 | 0.40 | 1.00 | 0.0 | 0.0 |
| `oracle_full` | 540 | 0.898 | 85.7% | 1.2 | 0.50 | 1.10 | 0.0 | 0.0 |

### Per target language (mean over source languages)

| Target | baseline cos / pitch-err st | vm_prosody cos / pitch-err st | vm_register cos / pitch-err st | oracle_full cos / pitch-err st |
| --- | --- | --- | --- | --- |
| de | 0.736 / 7.37 | 0.807 / 1.17 | 0.784 / 5.76 | 0.911 / 0.80 |
| en | 0.704 / 6.43 | 0.775 / 1.07 | 0.758 / 4.83 | 0.863 / 0.36 |
| es | 0.738 / 6.40 | 0.799 / 1.01 | 0.755 / 4.88 | 0.902 / 0.40 |
| hi | 0.752 / 6.22 | 0.827 / 1.04 | 0.781 / 4.74 | 0.919 / 0.35 |
| ja | 0.747 / 6.94 | 0.793 / 0.91 | 0.768 / 5.05 | 0.881 / 0.35 |
| ru | 0.767 / 6.02 | 0.818 / 1.35 | 0.791 / 4.55 | 0.913 / 0.75 |

### Per pair: vm_prosody − baseline (cosine Δ, pitch-error Δ st)

| Pair | Δ cosine | Δ pitch err | vm_prosody top-1 |
| --- | --- | --- | --- |
| de->en | +0.062 | -5.30 | 11% |
| de->es | +0.061 | -5.20 | 22% |
| de->hi | +0.061 | -5.03 | 22% |
| de->ja | +0.031 | -5.50 | 6% |
| de->ru | +0.049 | -4.65 | 11% |
| en->de | +0.075 | -6.50 | 17% |
| en->es | +0.070 | -6.10 | 17% |
| en->hi | +0.066 | -5.70 | 17% |
| en->ja | +0.049 | -6.60 | 17% |
| en->ru | +0.053 | -5.10 | 11% |
| es->de | +0.080 | -6.55 | 17% |
| es->en | +0.075 | -5.95 | 11% |
| es->hi | +0.090 | -5.65 | 22% |
| es->ja | +0.058 | -6.40 | 6% |
| es->ru | +0.061 | -4.97 | 11% |
| hi->de | +0.064 | -5.33 | 17% |
| hi->en | +0.080 | -5.20 | 17% |
| hi->es | +0.050 | -5.28 | 22% |
| hi->ja | +0.041 | -5.53 | 6% |
| hi->ru | +0.048 | -4.50 | 17% |
| ja->de | +0.060 | -6.30 | 11% |
| ja->en | +0.062 | -5.05 | 11% |
| ja->es | +0.057 | -5.07 | 11% |
| ja->hi | +0.072 | -4.55 | 17% |
| ja->ru | +0.044 | -4.10 | 11% |
| ru->de | +0.077 | -6.30 | 11% |
| ru->en | +0.078 | -5.25 | 11% |
| ru->es | +0.063 | -5.30 | 11% |
| ru->hi | +0.086 | -4.95 | 17% |
| ru->ja | +0.048 | -6.10 | 17% |

### vm_prosody tempo error by true speaker tempo

| True speed (wpm) | n | median tempo err (%) |
| --- | --- | --- |
| 140 | 180 | 31.2 |
| 175 | 180 | 20.0 |
| 205 | 180 | 6.7 |

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
| `vm_prosody` | 80 | 0.79 | 0.73 |
| `vm_register` | 80 | 0.84 | 0.86 |
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

- analyzer: 1.78 ms per audio second (Node v22.23.5, 4 vCPU, no GPU; 48 s at 22050 Hz)
- descriptor: 57 bytes JSON

Run: 2026-11-21T15:25:58, 29.9 min, eSpeak NG text-to-speech: 1.51  Data at: /usr/lib/x86_64-linux-gnu/espeak-ng-data
