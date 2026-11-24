# ADR-0009 — Dependencies and licensing

Status: Accepted (Phase 1) · Date: 2026-11-24

## Direct dependencies
| Crate | Used by | Why | License |
| --- | --- | --- | --- |
| serde (derive) | ce-core | serializable contracts | MIT OR Apache-2.0 |
| semver (serde) | ce-core | module/capability versions and requirements | MIT OR Apache-2.0 |
| uuid (serde, std) | ce-core | typed instance ids | Apache-2.0 OR MIT |
| uuid `v4` | runtime | default random id generator | Apache-2.0 OR MIT |
| tokio (rt, sync, time, macros) | runtime | control-plane runtime (ADR-0002) | MIT |
| tokio-util | runtime | `CancellationToken` | MIT |
| tracing (optional, feature `tracing`) | runtime | bridge to host logging | MIT |
| serde_json (dev) | tests | serialization tests | MIT OR Apache-2.0 |
| rtrb 0.4 (Phase 2) | ce-audio | wait-free SPSC rings for the callback handoff | MIT OR Apache-2.0 |
| cpal 0.18 (Phase 2, optional feature `cpal`) | ce-audio | cross-platform audio I/O (ADR-0010) | Apache-2.0 |

Rejected/avoided: `async-trait` (boxed futures suffice), `thiserror`/`anyhow`
(redaction needs custom `Display`), `futures` (not needed), `ringbuf` (rtrb
is smaller and purpose-built for real-time audio).

`cpal` and `rtrb` contain `unsafe` internally; CE's own crates remain
`#![forbid(unsafe_code)]`. On Linux the `cpal` feature links the system ALSA
library (LGPL-2.1 libasound, dynamically linked; build needs
`libasound2-dev`) — a distribution concern to review before shipping a
Linux binary SDK.

## Licensing policy
`deny.toml` allows MIT, Apache-2.0 and Unicode-3.0 only; anything
else requires review before a commercial SDK. The full transitive tree (all
features, all targets) was inspected: every crate offers one of these
licenses. The workspace crates themselves are marked `UNLICENSED` and
`publish = false` until the owner chooses a license.

## Toolchain
Pinned to Rust 1.97.0 (`rust-toolchain.toml`); MSRV declared as 1.85
(edition 2024) and verified with `cargo +1.85.0 check`.
