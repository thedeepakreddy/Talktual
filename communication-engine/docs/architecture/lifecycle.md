# Lifecycle

All three state machines are defined as explicit tables in `ce-core`
(`ModuleState::can_transition_to`, `RuntimeState::can_transition_to`,
`SessionState::can_transition_to`). Anything not in a table is rejected with a
`Lifecycle` error (`lifecycle.illegal_*_transition`). The tables are tested
exhaustively against an independently written matrix.

## Module

| From | Allowed next states | Trigger |
| --- | --- | --- |
| Discovered | Validated, Failed, Stopped | dependency + config validation at start; shutdown before start |
| Validated | Initialized, Failed, Stopped | `create` + `initialize` succeeded |
| Initialized | Ready, Stopping, Failed | whole graph initialized |
| Ready | Running, Stopping, Failed | `start` succeeded |
| Running | Stopping, Failed | shutdown; failure |
| Stopping | Stopped, Failed | `stop` finished (cleanly, with error, or forced) |
| Failed | Stopping | cleanup during shutdown or start failure |
| Stopped | — (terminal) | |

What happens at each step:

1. **Registration** (`Engine::register_module`, runtime `Configuring` only):
   descriptor self-checks, contract version, duplicate id, declared conflicts,
   exclusive-capability conflicts. A rejected module never enters the registry.
   Nothing is instantiated. Emits `ModuleRegistered`, `CapabilityRegistered`.
2. **Start** (`Engine::start`): `RuntimeStarting`; reject configuration for
   unregistered modules; resolve dependencies (missing, incompatible version,
   cycles) and compute a start order (providers first, ties in registration
   order); validate every module's configuration **before** creating any
   instance → `Validated`; then, in order, `create` + `initialize` →
   `Initialized`; all → `Ready`; then, in order, `start` → `Running`, followed
   by a health probe. `RuntimeStarted`.
3. **Failure during start**: the failing module → `Failed` (with a
   `RuntimeError` event); every module that already has an instance is stopped
   in reverse order; the runtime enters `Failed`. Only shutdown is possible.
4. **Shutdown**: `RuntimeStopping`; open sessions are closed; modules are
   stopped in reverse start order within the shutdown deadline (see below);
   the root cancellation token is cancelled; `RuntimeStopped`;
   `ShutdownCompleted` (always the last event); the core exits and every
   subscription then reports `Closed`.

### Stop semantics

For each module, `stop` is called with a budget of
`min(module_operation_timeout, remaining shutdown time)`. After `stop`
returns, the actor cancels the module's token and joins its supervised tasks.

| Outcome | Meaning |
| --- | --- |
| `Clean` | `stop` succeeded and all tasks ended |
| `StopFailed(report)` | `stop` (or a task during draining) returned an error; resources released anyway |
| `Forced` | `stop` or task draining exceeded the deadline; the actor task (and so every child task) was aborted |
| `AlreadyFailed` | the module had failed earlier; any remaining instance was terminated |
| `AlreadyStopped` | stopped cleanly earlier (cleanup of a failed start) |
| `NotStarted` | never instantiated |

`ShutdownReport.clean` is true only if every outcome is `Clean`,
`AlreadyStopped` or `NotStarted`. Events still queued in a module's mailbox
when `stop` is processed are discarded (lifecycle control has priority).

Dropping an `Engine` without `shutdown()` runs the same procedure in the
background with reason `HandleDropped`; it is bounded by the shutdown timeout.

## Runtime

`Configuring → Starting → Running → Stopping → Stopped`, plus
`Starting → Failed → Stopping` and `Configuring → Stopping`. Each change emits
`RuntimeStateChanged`.

## Session

`Created → Active → Closing → Closed`, with `Failed` reachable from the
non-terminal states and `Failed → Closed`. In Phase 1 a session becomes
`Active` immediately after creation (sessions carry no media). Sessions can be
created only while the runtime is `Running`; the count is capped by
`max_sessions`.
