// Outbound backpressure for committed units. One translation in flight; while
// it runs, later units of the SAME turn wait, and further units merge into the
// waiting one (bounded by maxUnitChars). Under a slow provider this caps the
// queue at one in-flight + one waiting per turn, so lag stops growing with the
// amount said, instead of every phrase queuing behind the previous one.

import type { Unit } from './segmenter';

export interface Pending {
  unit: Unit;
  /** When the (first) unit was committed. */
  committedAt: number;
  /** Ids of units merged into this one (their local records are folded in). */
  absorbed: string[];
}

export class OutboundQueue {
  private waiting: Pending[] = [];
  inFlight: Pending | null = null;
  merges = 0;
  maxDepth = 0;

  constructor(private readonly maxUnitChars: number) {}

  /** Returns the pending entry the unit ended up in, and whether it merged. */
  push(unit: Unit, now: number): { entry: Pending; merged: boolean } {
    const tail = this.waiting.at(-1);
    if (this.inFlight && tail && tail.unit.turn === unit.turn && tail.unit.text.length + 1 + unit.text.length <= this.maxUnitChars) {
      tail.unit = {
        ...tail.unit,
        text: `${tail.unit.text} ${unit.text}`,
        repaired: tail.unit.repaired || unit.repaired,
        uncertain: tail.unit.uncertain || unit.uncertain,
      };
      tail.absorbed.push(unit.id);
      this.merges++;
      return { entry: tail, merged: true };
    }
    const entry = { unit, committedAt: now, absorbed: [] };
    this.waiting.push(entry);
    this.maxDepth = Math.max(this.maxDepth, this.depth);
    return { entry, merged: false };
  }

  take(): Pending | null {
    if (this.inFlight) return null;
    this.inFlight = this.waiting.shift() ?? null;
    return this.inFlight;
  }

  done(): void {
    this.inFlight = null;
  }

  get depth(): number {
    return this.waiting.length + (this.inFlight ? 1 : 0);
  }

  clear(): void {
    this.waiting = [];
    this.inFlight = null;
  }
}

/** Smoothed translation latency → degraded flag (with hysteresis). */
export class LatencyMonitor {
  private ema: number | null = null;
  degraded = false;
  constructor(private readonly threshold: number) {}
  observe(ms: number): boolean {
    this.ema = this.ema === null ? ms : 0.6 * this.ema + 0.4 * ms;
    const was = this.degraded;
    if (!this.degraded && this.ema > this.threshold) this.degraded = true;
    else if (this.degraded && this.ema < this.threshold * 0.6) this.degraded = false;
    return was !== this.degraded;
  }
  get smoothedMs(): number | null {
    return this.ema === null ? null : Math.round(this.ema);
  }
  reset(): void {
    this.ema = null;
    this.degraded = false;
  }
}
