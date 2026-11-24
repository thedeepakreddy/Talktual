# Errors, health and recovery

## Error model

`CeError` = category + stable code + operational message + classified context
+ recoverability + optional (never rendered) source.

| Category | Typical codes | Default recoverability |
| --- | --- | --- |
| `invalid_config` | `config.out_of_range`, `config.inline_secret`, `config.unknown_module`, `config.unsupported_version` | Recoverable |
| `invalid_input` | `id.invalid_name`, `session.not_found`, `session.duplicate_language`, `event.unknown_session` | Recoverable |
| `lifecycle` | `runtime.not_running`, `runtime.registration_closed`, `lifecycle.illegal_*_transition` | Recoverable |
| `capability_resolution` | `registry.duplicate_module`, `registry.missing_dependency`, `registry.incompatible_version`, `registry.dependency_cycle`, `registry.exclusive_capability` | Recoverable |
| `queue_saturated` | `queue.saturated`, `session.limit_reached` | **Retryable** |
| `cancelled` | `module.cancelled` | Recoverable |
| `timeout` | `module.operation_timeout`, `provider.timeout` | **Retryable** (module op timeouts are marked Fatal for the module) |
| `provider` | `provider.unavailable`, `provider.rate_limited`, `provider.unauthorized`, … | per provider error kind |
| `serialization` | `event.unsupported_schema` | Recoverable |
| `internal` | `module.panicked`, `module.task_panicked`, `module.actor_exited` | Fatal |
| `shutdown` | `runtime.stopped` | Fatal |

`Recoverability` is `Retryable` (same call may succeed later), `Recoverable`
(fix the input; the component is still usable), `Fatal` (the component must be
restarted) or `Unknown` (treat as not retryable).

**Redaction.** `Display` and `Debug` render only operational context;
`Personal`, `Content`, `Biometric` and `Secret` values appear as
`[redacted:<class>]`. Sources are never rendered. `CeError::report(policy)`
produces the serializable `ErrorReport` that `RuntimeError` events carry.

**Provider mapping.** Adapters convert vendor failures into `ProviderError`
(`unavailable`, `rate_limited`, `timeout`, `unauthorized`, `invalid_request`,
`quota_exceeded`, `internal`), which maps to a `CeError` category, code,
recoverability and an implied health status.

## Supervision: failures are never swallowed

| Failure | Detection | Result |
| --- | --- | --- |
| lifecycle op returns `Err` | reply | module `Failed`, `RuntimeError`, start aborted with cleanup |
| lifecycle op exceeds timeout | core deadline | actor aborted, module `Failed` (`module.operation_timeout`) |
| panic in a lifecycle op | reply channel dropped | module `Failed` (`module.panicked`) |
| panic in `handle_event` | actor exit notification | module `Failed` (`module.panicked`) |
| supervised task returns `Err` / panics | actor `JoinSet` | module `Failed`, error carries the task name |
| task fails while draining at stop | stop result | `StopFailed` outcome |
| `handle_event` returns non-fatal `Err` | report | `RuntimeError`, module health `Degraded` |
| `handle_event` returns `Fatal` `Err` | report | module `Failed` |
| mailbox overflow under `FailModule` | dispatcher | module `Failed` (`module.mailbox_overflow`) |

## Health

`HealthStatus`: `Unknown`, `Healthy`, `Degraded(reason)`,
`Unhealthy(reason)`; orthogonal to lifecycle state. Module health comes from
`Module::health()` (probed after start and on `Engine::health()`), from
handler errors (`Degraded`) and failures (`Unhealthy`). Runtime health
aggregates: runtime `Failed` → Unhealthy; not `Running` → Unknown; any module
failed/unhealthy or degraded/unknown → Degraded; else Healthy. Every change
emits `HealthChanged`.

**No automatic failover, restart or retry in Phase 1.** A failed module stays
failed until the runtime is restarted; the runtime keeps serving the rest.
Health is probed on demand only (no periodic polling yet).
