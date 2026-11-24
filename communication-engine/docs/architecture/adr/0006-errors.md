# ADR-0006 — Error model

Status: Accepted (Phase 1) · Date: 2026-11-24

## Decision
A single cloneable `CeError` struct (not a large enum): `ErrorCategory`
(the spec's ten categories plus `invalid_input`), a stable string `code`, an
operational message, classified context fields, `Recoverability`
(`Retryable`/`Recoverable`/`Fatal`/`Unknown`, defaulted per category,
overridable) and an optional `Arc` source that is never rendered or
serialized. `ErrorReport` is the redacted serializable form used in events.

`InvalidInput` was added because command/argument validation errors are not
configuration errors. No `thiserror`/`anyhow` dependency: the hand-written
`Display`/`Debug` impls are what implement redaction.

## Consequences
Codes, not message text, are the matching contract for hosts and tests.
