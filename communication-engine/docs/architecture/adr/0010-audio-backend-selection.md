# ADR-0010 — Audio backend: cpal behind a CE-owned adapter boundary

Status: Accepted (Phase 2) · Date: 2026-11-24

## Context
Phase 2 needs real capture/playback on CE's plausible targets: iOS and
Android (Talktual's open decision DR-8.5-1 weighs a native shell), desktop
(macOS, Windows, Linux) and possibly a server host. The browser is out of
reach for a Rust runtime in Phase 2 (see ADR-0011, platform matrix). The
spec names CPAL as a candidate, not a decision.

## Options researched (crate metadata via crates.io, source read where relevant)
| Option | Coverage | License | Maintenance (observed) | Device/format API | Testability | Cost |
| --- | --- | --- | --- | --- | --- | --- |
| **cpal 0.18.2** | ALSA, PulseAudio/PipeWire/JACK (features), CoreAudio (macOS+iOS), WASAPI/ASIO, AAudio (Android), WebAudio (output) | Apache-2.0 | releases 2026-02, 06, 08; MSRV 1.85 | enumeration, stable `DeviceId`, supported config ranges, buffer-size ranges, typed errors (`DeviceNotAvailable`, `StreamInvalidated`, `DeviceChanged`, `Xrun`, `PermissionDenied`), stream clock timestamps | pure Rust on every target; ALSA null PCM usable in CI | one dependency, small tree on Linux (alsa, alsa-sys, libc, dasp_sample, bitflags, cfg-if) |
| Per-platform crates (`alsa` 0.12, `coreaudio-rs` 0.14, `wasapi` 0.27, `oboe` 0.6) | one OS each | MIT/Apache | varied; oboe-rs lags | full native control | four adapters to test | 4× adapter code and CI matrix |
| `cubeb` 0.38 (Mozilla) | broad | ISC | active (Firefox) | good | C++ library build | C++ toolchain in every build |
| PortAudio bindings 0.8 | broad | MIT | bindings stale | moderate | C library | system C dependency |
| `miniaudio` bindings 0.10 / `tinyaudio` 2.0 | broad / playback-centric | MIT | bindings stale / small scope | limited device detail | — | — |

## Decision
Use **cpal** (feature `cpal` of `ce-audio`, optional) as the first backend
adapter. CE contracts (`ce-core::audio`) and the `AudioBackend` trait stay
library-neutral: cpal types never leave `cpal_backend.rs`, and any of the
alternatives could be added as another adapter.

## Known cpal gaps, and how CE handles them
* **No hot-plug notifications** → CE re-enumerates on request, on backend
  notices and optionally on a poll interval, and diffs the catalog.
* **No permission query** → the adapter reports `NotRequiredByPlatform` on
  ALSA (no permission model) and `Unknown` elsewhere, which **denies
  capture** until the host supplies the platform state with
  `PermissionOverride`.
* **No layout information** (channel count only) → CE does not invent
  speaker positions (`ChannelLayout::Discrete`).
* 24-bit packed, 64-bit integer and DSD formats → skipped and counted.

## Verification (this phase)
* License/advisories: `cargo deny check` over the full all-target tree —
  advisories ok, licenses ok (MIT/Apache-2.0/Unicode-3.0 only).
* Compiles (`cargo check`) for x86_64 Linux, aarch64 iOS, aarch64 Android,
  x86_64 Windows, aarch64 macOS. **Only Linux was run**, against ALSA's
  virtual `null` PCM (no audio hardware in the environment).
* Real-device validation on each target is a Phase 3 entry task.
