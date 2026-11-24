# Phase 2 — Preliminary measurements

**No audio hardware was available.** Everything below was measured on
ALSA's virtual `null` PCM through the real cpal → CE path. It characterizes
CE's software pipeline under an extreme producer, **not** device latency,
stability or power. No performance targets are set from it.

## Method

* Tool: `cargo run --release -p communication-engine --features cpal --example audio_probe -- --device Discard --seconds 3 --rate R --channels C`
* Host: Ubuntu 24.04 container, Intel Xeon @ 2.10 GHz, 4 vCPU, 16 GB, no
  other load; Rust 1.97.0 release build; cpal 0.18.2 ALSA host; CE defaults
  (10 ms blocks, 250 ms capture ring, 64-block queue, 2 ms worker poll,
  `DropOldest`).
* The probe consumes blocks as fast as it can, records block inter-arrival
  times, counters from `close_audio`, and process CPU time and peak RSS from
  `/proc`.
* Caveat: the null PCM has **no clock** — it produces frames as fast as they
  are read — so the ring and queue saturate by design; "frames/expected"
  measures pipeline throughput, not device timing.

## Results (2026-11-24)

| Metric | 16 kHz mono | 48 kHz stereo |
| --- | --- | --- |
| enumerate | 3.78 ms | 5.76 ms |
| open (build stream) | 7.64 ms | 7.60 ms |
| start → first block | 2.26 ms | 4.14 ms |
| backend buffer (actual) | 80 frames | 240 frames |
| frames delivered in 3 s / real-time expectation | 157.7× | 841.2× |
| callbacks / frames per callback | 4,491,326 / 1.7 | 4,056,996 / 30.2 |
| block inter-arrival p50 / p99 / max | 0.00 / 2.13 / 9.40 ms | 0.00 / 0.02 / 5.75 ms |
| ring overflow frames (expected: null overruns) | 351,718,000 | 851,137,920 |
| queue-dropped blocks | 45 | 2,854 |
| discontinuities (gaps produced / flagged blocks seen) | 1,419 / 1,432 † | 16,188 / 16,816 † |
| max ring fill | 1000‰ | 1000‰ |
| process CPU during run | 103% of one core | 128% of one core |
| peak RSS | 6.2 MiB | 8.0 MiB |

† Measured before the counter fix described below.

## What these show

* Open/start costs on this host are single-digit milliseconds for the
  software path.
* Under a producer 150–840× faster than real time, CE stayed bounded
  (ring and queue at capacity, RSS < 10 MiB), kept delivering, and flagged
  every gap. The integration test `audio_cpal_alsa` asserts every
  sample-clock jump is flagged with its exact size.
* The run exposed a counting bug: delivery-queue drops were not counted as
  produced gaps, so flagged blocks could exceed the counter. Fixed before
  the report (the counter now includes queue drops).
* CPU ≈ 1 core is the null device spinning, not a CE property.

## Still needed (Phase 3 entry)

Run the same probe on real devices (iOS + AirPods, Android, macOS, Windows,
Linux with a sound card), at least 10 minutes per configuration, recording
device, OS, backend, format, buffer, CPU and battery; add loopback latency
measurement with a known stimulus. Only then set targets.
