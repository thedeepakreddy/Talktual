/**
 * The level behind the voice wave. Only real signals move it:
 *  - the recognizer reporting speech (a floor while it hears a voice),
 *  - each new interim word it reports (a bump that decays),
 *  - or, where the app really holds a microphone stream, its measured loudness.
 * Nothing reported → it decays to 0 and the wave lies flat. Never simulated.
 */
export class ActivityLevel {
  private target = 0;
  private bumpAt = 0;
  private floor = 0;
  private direct: number | null = null;
  private directAt = 0;
  /** Recent interim updates per second: busier speech → more waves. */
  private updates: number[] = [];

  constructor(
    private readonly decayMs = 650,
    private readonly now: () => number = () => (typeof performance !== 'undefined' ? performance.now() : Date.now())
  ) {}

  /** The recognizer reported new interim words (`chars` added). */
  bump(chars = 1): void {
    const t = this.now();
    const current = this.decayed(t);
    this.target = Math.min(1, Math.max(current, 0.45) + Math.min(0.35, 0.12 + chars * 0.012));
    this.bumpAt = t;
    this.updates.push(t);
  }

  /** Recognizer says a voice is (not) present right now. */
  setHearing(on: boolean): void {
    this.floor = on ? 0.22 : 0;
  }

  /** Measured loudness 0..1 from a real microphone stream. */
  setDirect(v: number): void {
    this.direct = Math.min(1, Math.max(0, v));
    this.directAt = this.now();
  }

  /** Nothing is being captured: everything goes to rest. */
  reset(): void {
    this.target = 0;
    this.floor = 0;
    this.direct = null;
    this.updates = [];
  }

  /** Level 0..1 at time `t`. */
  sample(t = this.now()): number {
    if (this.direct !== null && t - this.directAt < 250) return this.direct;
    return Math.max(this.floor, this.decayed(t));
  }

  /** 0..1: how busy the recognizer has been over the last 2 s. */
  busyness(t = this.now()): number {
    this.updates = this.updates.filter((u) => t - u < 2000);
    return Math.min(1, this.updates.length / 8);
  }

  private decayed(t: number): number {
    return this.target * Math.exp(-(t - this.bumpAt) / this.decayMs);
  }
}
