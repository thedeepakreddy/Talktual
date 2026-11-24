# ADR-0008 — Control plane vs. real-time media plane

Status: Accepted (Phase 1) · Date: 2026-11-24

## Decision
The Tokio-based runtime is the **control plane** only. Future audio
callbacks run on platform threads under the rules in realtime.md (no blocking,
allocation, locking, logging, I/O or awaiting; SPSC lock-free exchange;
atomic counters reported by control-plane tasks). No audio dependency (CPAL or
other) is added in Phase 1; CPAL remains a Phase 2 candidate to be validated
on real target devices.

## Consequences
Phase 1 makes no latency or real-time claims; tests are headless and run on
Linux x86_64 only.
