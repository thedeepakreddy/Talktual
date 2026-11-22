// Phase 6 — central processing policy. EVERY network-backed inference step
// (speech recognition, translation, live captions, speech output) asks this
// policy for a plan before running and re-checks at execution time. The policy
// never performs work itself; it decides and records (enum-only audit).
//
// Modes (user choice):
//   cloud      — Cloud-assisted: cloud providers under the existing consent; on-device when cloud can't be reached.
//   hybrid     — On-device first where supported; cloud only if consent + policy allow.
//   local-only — On-device only. Never cloud. If no on-device capability: fail closed (unavailable).
// Derived states (not choices):
//   offline    — network/providers unreachable: only on-device capability is available.
//   degraded   — at least one function is unavailable in the current mode/state (disclosed).

export type ProcessingMode = 'cloud' | 'hybrid' | 'local-only';
export const PROCESSING_MODES: ProcessingMode[] = ['cloud', 'hybrid', 'local-only'];

export type InferenceTask = 'asr' | 'translate' | 'caption' | 'tts';
export type Location = 'device' | 'cloud';
export type NetworkState = 'online' | 'offline' | 'providers-unreachable';

export type RouteReason =
  | 'cloud-mode'
  | 'device-preferred'
  | 'device-fallback' // cloud unreachable → on-device
  | 'cloud-fallback' // hybrid: no on-device capability, cloud allowed
  | 'local-only' // cloud blocked by mode
  | 'peer-requested-device' // the sender asked for on-device processing of their words
  | 'offline'
  | 'no-consent'
  | 'unavailable';

export interface RouteContext {
  /** An on-device capability exists for this task/language (coverage of a given input is only known by trying). */
  deviceAvailable: boolean;
  /** A cloud provider exists for this task (server capability). */
  cloudAvailable: boolean;
  /** Consent for the cloud route of this task (ASR consent; translation/tts follow the call consent). */
  cloudConsent?: boolean;
  /** The other person requested on-device processing for this content. */
  peerRequestedDevice?: boolean;
}

export interface RoutePlan {
  task: InferenceTask;
  /** Ordered steps to try; empty = unavailable (fail closed). */
  steps: Location[];
  reason: RouteReason;
}

export interface AuditEntry {
  t: number;
  task: InferenceTask;
  decision: 'plan' | 'blocked' | 'executed';
  location: Location | 'none';
  reason: RouteReason;
}

export class PolicyBlockedError extends Error {
  readonly code = 'policy-blocked';
  constructor(readonly task: InferenceTask, readonly reason: RouteReason) {
    super(`remote ${task} blocked by processing policy (${reason})`);
  }
}

export class ProcessingPolicy {
  private _mode: ProcessingMode;
  private _network: NetworkState = 'online';
  private audit: AuditEntry[] = [];
  /** Remote work in flight; aborted when the mode/network stops allowing cloud. */
  private remote = new Set<{ task: InferenceTask; controller: AbortController }>();
  private listeners = new Set<() => void>();
  counts = { blocked: 0, cloud: 0, device: 0, revoked: 0 };

  constructor(
    mode: ProcessingMode,
    private readonly now: () => number,
    private readonly maxAudit = 200
  ) {
    this._mode = PROCESSING_MODES.includes(mode) ? mode : 'cloud';
  }

  get mode(): ProcessingMode {
    return this._mode;
  }
  get network(): NetworkState {
    return this._network;
  }

  onChange(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  setMode(mode: ProcessingMode): void {
    if (!PROCESSING_MODES.includes(mode) || mode === this._mode) return;
    this._mode = mode;
    if (mode === 'local-only') this.revokeRemote('local-only');
    this.listeners.forEach((l) => l());
  }

  setNetwork(state: NetworkState): void {
    if (state === this._network) return;
    this._network = state;
    if (state === 'offline') this.revokeRemote('offline');
    this.listeners.forEach((l) => l());
  }

  /** May this task use the cloud at all right now (independent of capability)? */
  cloudPermitted(task: InferenceTask, ctx: Pick<RouteContext, 'cloudConsent' | 'peerRequestedDevice'> = {}): { ok: boolean; reason: RouteReason } {
    if (this._mode === 'local-only') return { ok: false, reason: 'local-only' };
    if (ctx.peerRequestedDevice) return { ok: false, reason: 'peer-requested-device' };
    if (this._network === 'offline') return { ok: false, reason: 'offline' };
    if (task === 'asr' && ctx.cloudConsent === false) return { ok: false, reason: 'no-consent' };
    return { ok: true, reason: 'cloud-mode' };
  }

  /** Same as plan() but not audited (for status display). */
  preview(task: InferenceTask, ctx: RouteContext): RoutePlan {
    const saved = this.audit.length;
    const blocked = this.counts.blocked;
    const p = this.plan(task, ctx);
    this.audit.length = saved;
    this.counts.blocked = blocked;
    return p;
  }

  plan(task: InferenceTask, ctx: RouteContext): RoutePlan {
    const cloud = this.cloudPermitted(task, ctx);
    const cloudOk = cloud.ok && ctx.cloudAvailable && this._network !== 'providers-unreachable';
    let plan: RoutePlan;
    if (!cloud.ok) {
      plan = { task, steps: ctx.deviceAvailable ? ['device'] : [], reason: cloud.reason };
    } else if (this._mode === 'hybrid') {
      plan = ctx.deviceAvailable
        ? { task, steps: cloudOk ? ['device', 'cloud'] : ['device'], reason: 'device-preferred' }
        : { task, steps: cloudOk ? ['cloud'] : [], reason: cloudOk ? 'cloud-fallback' : 'unavailable' };
    } else {
      // cloud-assisted
      plan = cloudOk
        ? { task, steps: ctx.deviceAvailable ? ['cloud', 'device'] : ['cloud'], reason: 'cloud-mode' }
        : { task, steps: ctx.deviceAvailable ? ['device'] : [], reason: ctx.deviceAvailable ? 'device-fallback' : this._network === 'online' ? 'unavailable' : 'offline' };
    }
    this.record(task, plan.steps.length ? 'plan' : 'blocked', plan.steps[0] ?? 'none', plan.reason);
    if (!plan.steps.length) this.counts.blocked++;
    return plan;
  }

  /**
   * Execution-time gate for a REMOTE call. Throws PolicyBlockedError if the
   * cloud is not permitted now (mode/network may have changed since planning).
   * Returns a controller that is aborted if permission is later revoked.
   */
  beginRemote(task: InferenceTask, ctx: Pick<RouteContext, 'cloudConsent' | 'peerRequestedDevice'> = {}): { signal: AbortSignal; end: () => void } {
    const p = this.cloudPermitted(task, ctx);
    if (!p.ok) {
      this.counts.blocked++;
      this.record(task, 'blocked', 'cloud', p.reason);
      throw new PolicyBlockedError(task, p.reason);
    }
    this.counts.cloud++;
    this.record(task, 'executed', 'cloud', 'cloud-mode');
    const entry = { task, controller: new AbortController() };
    this.remote.add(entry);
    return { signal: entry.controller.signal, end: () => this.remote.delete(entry) };
  }

  noteDevice(task: InferenceTask, reason: RouteReason): void {
    this.counts.device++;
    this.record(task, 'executed', 'device', reason);
  }

  get remoteInFlight(): number {
    return this.remote.size;
  }

  private revokeRemote(reason: RouteReason): void {
    for (const r of this.remote) {
      r.controller.abort();
      this.counts.revoked++;
      this.record(r.task, 'blocked', 'cloud', reason);
    }
    this.remote.clear();
  }

  private record(task: InferenceTask, decision: AuditEntry['decision'], location: AuditEntry['location'], reason: RouteReason): void {
    this.audit.push({ t: this.now(), task, decision, location, reason });
    if (this.audit.length > this.maxAudit) this.audit.shift();
  }

  /** Enum-only audit trail (no content). */
  auditLog(): AuditEntry[] {
    return [...this.audit];
  }

  clearAudit(): void {
    this.audit = [];
  }
}

/** Combine two signals (session cancellation + policy revocation). */
export function anySignal(a: AbortSignal | undefined, b: AbortSignal): AbortSignal {
  if (!a) return b;
  const c = new AbortController();
  const abort = () => c.abort();
  if (a.aborted || b.aborted) c.abort();
  a.addEventListener('abort', abort, { once: true });
  b.addEventListener('abort', abort, { once: true });
  return c.signal;
}
