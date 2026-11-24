# ADR-0007 — Privacy classification and redaction by default

Status: Accepted (Phase 1) · Date: 2026-11-24

## Decision
* Ordered `DataClass` (public … secret) on every event, error context field
  and log field. Default `RedactionPolicy` shows `operational` only; `secret`
  is never shown under any policy.
* The runtime redacts before calling the host's `ObservabilitySink`; the sink
  never receives unredacted values. Module attributes are never logged.
* Configuration cannot contain inline secrets (`SecretRef` references only).
* Retention, consent and erasure are represented (`RetentionPolicy` without a
  "forever" variant, `ConsentPurpose`/`ConsentState`, `DataErasure`), not yet
  enforced, because Phase 1 processes no data. Enforcement is a Phase 2 entry
  criterion for any capture or remote processing.
* Voice-profile work is reserved behind explicit governance (privacy.md).
