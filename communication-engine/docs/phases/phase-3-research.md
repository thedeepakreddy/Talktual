# Phase 3 research — detector and approach survey

Date: 2026-11-24. Method: candidates were reviewed (license, maintenance,
platform/rate support, dependencies, unsafe/FFI, MSRV), and the two that
could be built without unapproved downloads were **benchmarked through CE's
own `VoiceActivityDetector` adapter trait, stabilizer and scoring** on the
held-out test split. The benchmark harness lived in a scratch crate outside
the workspace (it is not part of CE and added no dependency to it); its
adapter code is reproduced below.

## Candidates

| Candidate | License | Kind | Rates | Notes | Outcome |
|---|---|---|---|---|---|
| CE `ce.vad.spectral` | CE | adaptive floor + spectral features + logistic | 8–96 kHz | no deps, no unsafe, deterministic | **default** |
| CE `ce.vad.energy` | CE | adaptive floor SNR | 8–96 kHz | cheapest (RTF ≈ 0.0002) | alternative / degraded fallback |
| webrtc-vad 0.4.0 | MIT | GMM (C, FFI) | 8/16/32/48 kHz, 10/20/30 ms | binary output; `!Send` state; C build | benchmarked; rejected |
| earshot 1.2.2 | MIT OR Apache-2.0 | small neural net, pure Rust, 40 KB weights | 16 kHz, 256-sample frames | `unsafe` inside; MSRV 1.87 | benchmarked; future optional adapter |
| nnnoiseless 0.5.2 | BSD-3-Clause | RNNoise port (denoiser + VAD prob.) | 48 kHz | denoising is out of scope | reviewed only |
| Silero VAD / TEN VAD | MIT / Apache-2.0 | neural, ONNX | 8/16 kHz | needs ONNX Runtime native binary | not evaluated (download not authorized) |

## Benchmark (test split, 16 kHz mono, default CE stabilizer and segment policy)

Columns as in the evaluation report; "merges" > 0 with precision ≈ 0.64
means one region spanned the whole stream (always on).

| detector | condition | P | R | F1 | merges | frag | endpoints | false EP | onset err | offset err | RTF |
|---|---|---|---|---|---|---|---|---|---|---|---|
| ce.vad.spectral | clean | 0.966 | 1.000 | 0.983 | 1 | 1.00 | 6/6 | 0 | 364 | 230 | 0.0065 |
| ce.vad.energy | clean | 0.976 | 1.000 | 0.988 | 1 | 1.00 | 6/6 | 0 | 371 | 194 | 0.0001 |
| webrtc-vad (quality) | clean | 0.910 | 1.000 | 0.953 | 1 | 1.00 | 6/6 | 1 | 420 | 341 | 0.0003 |
| webrtc-vad (aggressive) | clean | 0.953 | 0.999 | 0.976 | 1 | 1.00 | 6/6 | 0 | 360 | 284 | 0.0003 |
| earshot | clean | 0.956 | 0.987 | 0.971 | 1 | 1.00 | 6/6 | 0 | 379 | 280 | 0.0007 |
| ce.vad.spectral | white 20 dB | 0.987 | 0.963 | 0.975 | 1 | 1.00 | 6/6 | 0 | 379 | 231 | 0.0066 |
| ce.vad.energy | white 20 dB | 0.987 | 0.914 | 0.949 | 1 | 1.29 | 5/6 | 1 | 376 | 277 | 0.0002 |
| webrtc-vad (quality) | white 20 dB | 0.891 | 0.995 | 0.940 | 1 | 1.00 | 3/6 | 1 | 439 | 297 | 0.0004 |
| webrtc-vad (aggressive) | white 20 dB | 0.969 | 0.982 | 0.976 | 1 | 1.00 | 6/6 | 1 | 364 | 257 | 0.0004 |
| earshot | white 20 dB | 0.955 | 0.977 | 0.966 | 1 | 1.00 | 6/6 | 0 | 403 | 281 | 0.0008 |
| ce.vad.spectral | white 10 dB | 1.000 | 0.653 | 0.790 | 0 | 1.20 | 4/6 | 2 | 188 | 166 | 0.0067 |
| ce.vad.energy | white 10 dB | 1.000 | 0.624 | 0.769 | 0 | 1.00 | 2/6 | 1 | 27 | 197 | 0.0002 |
| webrtc-vad (quality) | white 10 dB | 0.642 | 1.000 | 0.782 | 1 | 1.00 | 0/6 | 0 | 12314 | 13054 | 0.0003 |
| webrtc-vad (aggressive) | white 10 dB | 0.953 | 0.969 | 0.961 | 1 | 1.00 | 6/6 | 0 | 373 | 261 | 0.0003 |
| earshot | white 10 dB | 0.856 | 0.967 | 0.908 | 1 | 1.00 | 4/6 | 2 | 1414 | 1213 | 0.0010 |
| ce.vad.spectral | white 5 dB | 1.000 | 0.595 | 0.746 | 0 | 1.33 | 2/6 | 1 | 43 | 187 | 0.0065 |
| ce.vad.energy | white 5 dB | 1.000 | 0.146 | 0.255 | 0 | 1.00 | 1/6 | 1 | 35 | 2760 | 0.0002 |
| webrtc-vad (quality) | white 5 dB | 0.642 | 1.000 | 0.782 | 1 | 1.00 | 0/6 | 0 | 12314 | 13054 | 0.0003 |
| webrtc-vad (aggressive) | white 5 dB | 0.642 | 1.000 | 0.782 | 1 | 1.00 | 0/6 | 0 | 12314 | 13054 | 0.0003 |
| earshot | white 5 dB | 0.820 | 0.969 | 0.888 | 1 | 1.00 | 3/6 | 1 | 2579 | 1869 | 0.0007 |
| ce.vad.spectral | pink 10 dB | 0.891 | 0.959 | 0.923 | 1 | 1.14 | 3/6 | 3 | 379 | 449 | 0.0060 |
| ce.vad.energy | pink 10 dB | 1.000 | 0.625 | 0.769 | 0 | 1.25 | 3/6 | 1 | 28 | 165 | 0.0001 |
| webrtc-vad (quality) | pink 10 dB | 0.642 | 1.000 | 0.782 | 1 | 1.00 | 0/6 | 0 | 12314 | 13054 | 0.0003 |
| webrtc-vad (aggressive) | pink 10 dB | 0.655 | 1.000 | 0.791 | 1 | 1.00 | 0/6 | 0 | 7360 | 9521 | 0.0003 |
| earshot | pink 10 dB | 0.827 | 0.978 | 0.896 | 1 | 1.00 | 3/6 | 1 | 2607 | 1874 | 0.0008 |
| ce.vad.spectral | music 10 dB | 0.643 | 1.000 | 0.782 | 1 | 1.00 | 0/6 | 0 | 12284 | 13054 | 0.0061 |
| ce.vad.energy | music 10 dB | 0.779 | 0.838 | 0.807 | 1 | 1.14 | 2/6 | 1 | 3277 | 5863 | 0.0001 |
| webrtc-vad (quality) | music 10 dB | 0.642 | 1.000 | 0.782 | 1 | 1.00 | 0/6 | 0 | 12314 | 13054 | 0.0002 |
| webrtc-vad (aggressive) | music 10 dB | 0.648 | 1.000 | 0.787 | 1 | 1.00 | 0/6 | 0 | 11964 | 13054 | 0.0002 |
| earshot | music 10 dB | 0.643 | 1.000 | 0.783 | 1 | 1.00 | 0/6 | 0 | 12274 | 13054 | 0.0008 |
| ce.vad.spectral | clipped +20 dB | 0.898 | 1.000 | 0.946 | 1 | 1.00 | 6/6 | 0 | 549 | 303 | 0.0061 |
| ce.vad.energy | clipped +20 dB | 0.901 | 1.000 | 0.948 | 1 | 1.00 | 6/6 | 0 | 563 | 276 | 0.0001 |
| webrtc-vad (quality) | clipped +20 dB | 0.875 | 1.000 | 0.933 | 1 | 1.00 | 6/6 | 1 | 563 | 357 | 0.0003 |
| webrtc-vad (aggressive) | clipped +20 dB | 0.910 | 1.000 | 0.953 | 1 | 1.00 | 6/6 | 0 | 471 | 304 | 0.0003 |
| earshot | clipped +20 dB | 0.945 | 0.988 | 0.966 | 1 | 1.00 | 6/6 | 0 | 377 | 310 | 0.0007 |
| ce.vad.spectral | quiet -30 dB | 1.000 | 0.931 | 0.964 | 0 | 1.29 | 6/6 | 0 | 29 | 66 | 0.0062 |
| ce.vad.energy | quiet -30 dB | 0.987 | 0.960 | 0.974 | 1 | 1.00 | 6/6 | 0 | 373 | 241 | 0.0002 |
| webrtc-vad (quality) | quiet -30 dB | 0.962 | 0.994 | 0.978 | 1 | 1.00 | 6/6 | 1 | 367 | 253 | 0.0003 |
| webrtc-vad (aggressive) | quiet -30 dB | 0.978 | 0.978 | 0.978 | 1 | 1.00 | 6/6 | 0 | 376 | 246 | 0.0003 |
| earshot | quiet -30 dB | 0.931 | 0.631 | 0.752 | 0 | 1.00 | 2/6 | 2 | 355 | 588 | 0.0007 |

### Reading the results

* **Clean / quiet speech**: all detectors are close (F1 0.95–0.99) except
  earshot on quiet input (−30 dB: recall 0.63, 4 of 6 endpoints missed).
* **White noise**: webrtc-vad (aggressive) is best at 10–20 dB but collapses
  to always-on at 5 dB; earshot degrades most gracefully; CE spectral keeps
  precision 1.0 but loses recall at ≤ 10 dB.
* **Pink noise**: CE spectral is best (0.923); webrtc-vad is always on.
* **Music**: every detector, including CE's, reports sustained music as
  activity. CE's energy detector is least bad because its floor adapts.
* **Cost** (real-time factor, one core, release build): CE energy 0.0002,
  webrtc 0.0003, earshot 0.0007–0.0010, CE spectral ≈ 0.006 (dominated by a
  1024-point FFT per hop; still 160× faster than real time).

Decision: ADR-0015.

## Adapter code used for the benchmark (scratch crate)

```rust
struct Earshot { d: Box<earshot::Detector>, buf: Vec<f32>, last: f32 }
impl VoiceActivityDetector for Earshot {
    fn needs_audio(&self) -> bool { true }
    fn score_audio(&mut self, _: &Hop, mono: &[f32], _rate: u32) -> VadObservation {
        self.buf.extend_from_slice(mono);
        while self.buf.len() >= 256 {
            let f: Vec<f32> = self.buf.drain(..256).map(|v| v.clamp(-1.0, 1.0)).collect();
            self.last = self.d.predict_f32(&f);
        }
        VadObservation { score: self.last, settled: true }
    }
    // descriptor / reset / needs_spectral / score omitted
}
// webrtc-vad: each 10 ms hop → i16 → Vad::is_voice_segment → 1.0 / 0.0.
```

## Other approaches considered

* **Segment/endpoint logic**: hysteresis with backdated boundaries plus
  tiered endpoint candidates was chosen over a fixed silence timeout alone
  (no early signal) and over model-based end-of-turn prediction (semantic,
  out of scope).
* **Noise floor**: asymmetric tracker (fast down, 3 dB/s up) plus a
  quality-side p10 of inactive hops; minimum statistics would be more
  robust in non-stationary noise but costs more state; revisit with real
  recordings.
* **Quality**: objective signal measures only (level, clipping, SNR
  estimate, DC, imbalance, dropouts). Perceptual/intrusive measures (PESQ,
  POLQA, DNSMOS) need references or models and licenses; out of scope.
