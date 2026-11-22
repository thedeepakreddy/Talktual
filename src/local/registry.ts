// Runtime registry: discovery, bounded loading, concurrency, timeouts and
// output validation. Prefers responsive degradation over loading everything.

import { base, type AsrRuntime, type LocalTranslateRequest, type LocalTranslateResult, type Readiness, type RuntimeInfo, type TranslateRuntime, type VoiceRuntime } from './types';

export interface RegistryLimits {
  /** Translate runtimes kept loaded at once (LRU unload beyond). */
  maxLoaded: number;
  /** Concurrent local calls per runtime. */
  concurrency: number;
  /** Waiting local calls per runtime; beyond this the call is refused (caller falls back). */
  maxQueue: number;
  /** Per-call time bound. */
  callTimeoutMs: number;
  /** Discovery time bound per runtime. */
  discoverTimeoutMs: number;
  /** Unload a runtime after this long unused. */
  idleUnloadMs: number;
  maxOutputChars: number;
}

export const DEFAULT_REGISTRY_LIMITS: RegistryLimits = {
  maxLoaded: 2,
  concurrency: 1,
  maxQueue: 4,
  callTimeoutMs: 1500,
  discoverTimeoutMs: 1500,
  idleUnloadMs: 60_000,
  maxOutputChars: 2000,
};

export interface RegistryClock {
  now(): number;
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(t: unknown): void;
}

export type LocalCallOutcome =
  | { ok: true; text: string; runtime: string; ms: number; coverage: 'exact' | 'template' }
  | { ok: false; reason: 'no-runtime' | 'not-covered' | 'busy' | 'timeout' | 'error' | 'invalid-output' | 'cancelled' };

interface Slot {
  rt: TranslateRuntime;
  readiness: Readiness;
  loaded: boolean;
  lastUsed: number;
  active: number;
  waiting: Array<() => void>;
  idleTimer: unknown;
}

const CONTROL = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/;

/** Output validation: non-empty, bounded, no control characters, not wildly longer than input. */
export function validLocalOutput(input: string, output: string, max: number): boolean {
  if (typeof output !== 'string') return false;
  const t = output.trim();
  return t.length > 0 && t.length <= max && t.length <= input.length * 4 + 60 && !CONTROL.test(t);
}

export class RuntimeRegistry {
  private translators: Slot[] = [];
  private voices: VoiceRuntime[] = [];
  private asr: AsrRuntime[] = [];
  private voiceReady = new Map<string, Readiness>();
  private asrReady = new Map<string, Readiness>();
  private disposed = false;
  stats = { calls: 0, covered: 0, misses: 0, refusedBusy: 0, timeouts: 0, invalid: 0, loads: 0, unloads: 0 };

  constructor(
    private readonly clock: RegistryClock,
    readonly limits: RegistryLimits = DEFAULT_REGISTRY_LIMITS
  ) {}

  addTranslator(rt: TranslateRuntime): void {
    this.translators.push({ rt, readiness: 'unknown', loaded: false, lastUsed: 0, active: 0, waiting: [], idleTimer: null });
  }
  addVoice(rt: VoiceRuntime): void {
    this.voices.push(rt);
  }
  addAsr(rt: AsrRuntime): void {
    this.asr.push(rt);
  }

  /** Time-bounded discovery of every runtime; a hanging platform API reports 'unknown', never blocks. */
  async discover(): Promise<RuntimeInfo[]> {
    const bound = <T,>(p: Promise<T>, fallback: T) =>
      Promise.race([p.catch(() => fallback), new Promise<T>((r) => this.clock.setTimeout(() => r(fallback), this.limits.discoverTimeoutMs))]);
    await Promise.all([
      ...this.translators.map(async (s) => (s.readiness = await bound(s.rt.discover(this.limits.discoverTimeoutMs), 'unknown' as Readiness))),
      ...this.voices.map(async (v) => this.voiceReady.set(v.id, await bound(v.discover(this.limits.discoverTimeoutMs), 'unknown' as Readiness))),
      ...this.asr.map(async (a) => this.asrReady.set(a.id, await bound(a.discover(this.limits.discoverTimeoutMs), 'unknown' as Readiness))),
    ]);
    return this.infos();
  }

  infos(): RuntimeInfo[] {
    return [
      ...this.translators.map((s) => ({ ...s.rt.info(), readiness: s.readiness })),
      ...this.voices.map((v) => ({ ...v.info(), readiness: this.voiceReady.get(v.id) ?? 'unknown' })),
      ...this.asr.map((a) => ({ ...a.info(), readiness: this.asrReady.get(a.id) ?? 'unknown' })),
    ];
  }

  /** Is there a READY local translator for this pair? (Coverage of a given phrase is only known by trying.) */
  canTranslate(from: string, to: string): boolean {
    return this.translators.some((s) => s.readiness === 'ready' && s.rt.supports(from, to));
  }

  hasLocalVoice(lang: string): boolean {
    return this.voices.some((v) => this.voiceReady.get(v.id) === 'ready' && v.hasLocalVoice(lang));
  }

  hasLocalAsr(lang: string): boolean {
    return this.asr.some((a) => this.asrReady.get(a.id) === 'ready' && a.supports(base(lang)));
  }

  async translate(req: LocalTranslateRequest): Promise<LocalCallOutcome> {
    if (this.disposed) return { ok: false, reason: 'no-runtime' };
    const slots = this.translators.filter((s) => s.readiness === 'ready' && s.rt.supports(req.from, req.to));
    if (!slots.length) return { ok: false, reason: 'no-runtime' };
    let last: LocalCallOutcome = { ok: false, reason: 'not-covered' };
    for (const s of slots) {
      last = await this.callOne(s, req);
      if (last.ok || ('reason' in last && last.reason === 'cancelled')) return last;
    }
    return last;
  }

  private async callOne(s: Slot, req: LocalTranslateRequest): Promise<LocalCallOutcome> {
    this.stats.calls++;
    if (req.signal?.aborted) return { ok: false, reason: 'cancelled' };
    if (s.active >= this.limits.concurrency) {
      if (s.waiting.length >= this.limits.maxQueue) {
        this.stats.refusedBusy++;
        return { ok: false, reason: 'busy' };
      }
      await new Promise<void>((r) => s.waiting.push(r));
    }
    s.active++;
    const t0 = this.clock.now();
    try {
      if (!s.loaded) {
        await s.rt.load(req.signal);
        s.loaded = true;
        this.stats.loads++;
        this.enforceLoadedBound(s);
      }
      s.lastUsed = this.clock.now();
      this.armIdle(s);
      let timer: unknown = null;
      const out = await Promise.race<LocalTranslateResult | 'timeout'>([
        s.rt.translate(req),
        new Promise((r) => (timer = this.clock.setTimeout(() => r('timeout'), this.limits.callTimeoutMs))),
      ]);
      this.clock.clearTimeout(timer);
      if (req.signal?.aborted) return { ok: false, reason: 'cancelled' };
      if (out === 'timeout') {
        this.stats.timeouts++;
        return { ok: false, reason: 'timeout' };
      }
      if (out === null) {
        this.stats.misses++;
        return { ok: false, reason: 'not-covered' };
      }
      if (!validLocalOutput(req.text, out.text, this.limits.maxOutputChars)) {
        this.stats.invalid++;
        return { ok: false, reason: 'invalid-output' };
      }
      this.stats.covered++;
      return { ok: true, text: out.text.trim(), runtime: s.rt.id, ms: this.clock.now() - t0, coverage: out.coverage };
    } catch {
      return req.signal?.aborted ? { ok: false, reason: 'cancelled' } : { ok: false, reason: 'error' };
    } finally {
      s.active--;
      s.waiting.shift()?.();
    }
  }

  private enforceLoadedBound(keep: Slot): void {
    const candidates = this.translators.filter((s) => s.loaded && s !== keep && !s.active).sort((a, b) => a.lastUsed - b.lastUsed);
    while (this.loadedCount > this.limits.maxLoaded && candidates.length) this.unloadSlot(candidates.shift()!);
  }

  private armIdle(s: Slot): void {
    if (s.idleTimer !== null) this.clock.clearTimeout(s.idleTimer);
    s.idleTimer = this.clock.setTimeout(() => {
      s.idleTimer = null;
      if (!s.active) this.unloadSlot(s);
    }, this.limits.idleUnloadMs);
  }

  private unloadSlot(s: Slot): void {
    if (!s.loaded) return;
    s.rt.unload();
    s.loaded = false;
    this.stats.unloads++;
  }

  /** Memory pressure / mode change: unload everything; reloading is lazy. */
  unloadAll(): void {
    for (const s of this.translators) {
      if (s.idleTimer !== null) this.clock.clearTimeout(s.idleTimer);
      s.idleTimer = null;
      if (!s.active) this.unloadSlot(s);
    }
  }

  get loadedCount(): number {
    return this.translators.filter((s) => s.loaded).length;
  }

  dispose(): void {
    this.unloadAll();
    this.translators.forEach((s) => s.rt.dispose());
    this.disposed = true;
  }
}
