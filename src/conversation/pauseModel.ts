// Adaptive hold window. Learns how long THIS speaker typically pauses between
// recognizer finals inside a thought, and holds an incomplete-looking fragment
// a little longer than that — always clamped to [minHoldMs, maxHoldMs].
// Only timing numbers are kept (no content); cleared with the session.

export class AdaptiveHold {
  private ema: number | null = null;
  private samples = 0;
  constructor(
    private readonly min: number,
    private readonly max: number,
    private readonly fallback: number
  ) {}

  /** A gap (ms) between a held fragment's final and the speaker continuing. */
  observeGap(ms: number): void {
    if (!Number.isFinite(ms) || ms < 0 || ms > 10_000) return;
    this.ema = this.ema === null ? ms : 0.7 * this.ema + 0.3 * ms;
    this.samples++;
  }

  holdMs(): number {
    const base = this.ema === null || this.samples < 2 ? this.fallback : this.ema * 1.5 + 200;
    return Math.round(Math.min(this.max, Math.max(this.min, base)));
  }

  reset(): void {
    this.ema = null;
    this.samples = 0;
  }
}
