# ADR-0003 — Module contract: factory + instance, sequential actor, boxed futures

Status: Accepted (Phase 1) · Date: 2026-11-24

## Decision
* **`ModuleFactory`** (registered): cheap static `descriptor()`,
  `validate_config()`, `create()`. **`Module`** (instance): `initialize`,
  `start`, `stop`, `health`, `handle_event`. Discovery reads descriptors only,
  so it can never instantiate models or devices (tested).
* Methods take `&mut self` and are invoked **sequentially by the module's
  actor**; module authors need no internal locking. Lifecycle control has
  priority over event delivery.
* Async methods return `BoxFuture<'a, Result<…>>` (a type alias for
  `Pin<Box<dyn Future + Send + 'a>>`). Native `async fn` in traits is not
  object-safe, and modules are stored as `Box<dyn Module>`. This avoids the
  `async-trait` macro dependency at the cost of `Box::pin(async move {..})` in
  implementations.
* Modules depend on **capabilities** (id + semver requirement, optional or
  not), never on other modules' ids; conflicts are declared by module id;
  capabilities are `Shared` or `Exclusive`.
* A `ContractVersion {major, minor}` (now 1.0) is checked at registration:
  same major, module minor ≤ runtime minor.
* Background work only through `ModuleContext::spawn` (supervised, bounded,
  joined at stop).

## Consequences
* One boxed allocation per lifecycle call or event — acceptable on the
  control plane, never used on a real-time path.
* Dynamic plugin loading is out of scope; factories are linked statically.
