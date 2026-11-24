# ADR-0013 — Capture authorization gate

Status: Accepted (Phase 2) · Date: 2026-11-24

## Decision
Capture requires **CE consent** (`ConsentPurpose::Capture` granted on the
session, younger than `consent_max_age_ms` if configured) **and platform
permission** (`Granted` or `NotRequiredByPlatform`). Everything else denies.
Device open is never treated as authorization.

Enforcement points:
1. `open_capture` — before any device is touched (nothing opens on denial).
2. `start_audio` — re-evaluated (consent can go stale; permission can change).
3. Continuously — on `UpdateConsent`, on backend `PermissionChanged`
   notices, on a backend `PermissionRevoked` stream fault, and on every
   housekeeping tick (catches silent permission changes and staleness).

Revocation is two-phase: **synchronously** the callback gate closes, queued
blocks are purged and readers fail with `Revoked`; **then** the device is
released (stream closed). Revoked streams never restart; a new stream needs
fresh authorization.

## Platform enforcement status
CE enforces the gate on every platform, but the *platform permission input*
is only as good as its source: cpal cannot query it, so on non-ALSA platforms
the host must supply it via `PermissionOverride` or capture stays denied.
**No per-platform permission integration was implemented or verified in
Phase 2.**
