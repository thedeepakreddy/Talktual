// Streaming, bounded-memory voice analyzer (on-device).
//
// Feed it audio chunks as they arrive; it keeps only fixed-size statistics:
//   * a pitch histogram (quarter-semitone bins, 55–550 Hz) from YIN on 40 ms
//     frames hopped every 20 ms,
//   * counters for voiced/speech time and energy-envelope peaks (syllable rate).
// No audio is retained beyond the current frame. Memory is O(1) regardless of
// how long someone talks. Plain erasable TypeScript: runs in browsers and in
// Node (`node --experimental-strip-types`) for the evaluation harness.

import type { VoiceProfile } from './types.ts';

const F0_MIN = 55;
const F0_MAX = 550;
const BINS_PER_SEMITONE = 4;
const N_BINS = Math.ceil(12 * Math.log2(F0_MAX / F0_MIN) * BINS_PER_SEMITONE) + 1; // ~160
const TARGET_RATE = 8000; // analysis rate after decimation
const FRAME_S = 0.04;
const HOP_S = 0.02;
const ENV_HOP_S = 0.01;
const YIN_THRESHOLD = 0.15;

const semis = (hz: number) => 12 * Math.log2(hz / F0_MIN);

export interface AnalyzerOptions {
  /** Minimum voiced seconds before confidence can reach 1. */
  fullConfidenceVoicedS?: number;
}

export class VoiceAnalyzer {
  private readonly hist = new Float64Array(N_BINS);
  private voicedFrames = 0;
  private totalFrames = 0;
  private periodicitySum = 0;

  // Decimation state
  private readonly factor: number;
  private readonly rate: number;
  private accum = 0;
  private accumN = 0;
  private readonly frameLen: number;
  private readonly hopLen: number;
  private readonly ring: Float32Array;
  private ringFill = 0;
  private sinceHop = 0;

  // Energy envelope / syllable peaks
  private readonly envHop: number;
  private envAcc = 0;
  private envN = 0;
  private envPrev1 = 0;
  private envSmooth = 0;
  private envTrough = Infinity;
  private wasRising = false;
  private envFloor = 1e-4;
  private envPeak = 0;
  private speechEnvFrames = 0;
  private peaks = 0;
  private framesSincePeak = 1e9;

  private readonly fullConfidenceVoicedS: number;

  constructor(sampleRate: number, opts: AnalyzerOptions = {}) {
    if (!(sampleRate >= 8000)) throw new Error('sampleRate must be >= 8000');
    this.factor = Math.max(1, Math.round(sampleRate / TARGET_RATE));
    this.rate = sampleRate / this.factor;
    this.frameLen = Math.round(FRAME_S * this.rate);
    this.hopLen = Math.round(HOP_S * this.rate);
    this.envHop = Math.round(ENV_HOP_S * this.rate);
    this.ring = new Float32Array(this.frameLen);
    this.fullConfidenceVoicedS = opts.fullConfidenceVoicedS ?? 2;
  }

  /** Feed mono samples in [-1, 1]. */
  push(chunk: Float32Array): void {
    for (let i = 0; i < chunk.length; i++) {
      // Box-filter decimation (crude low-pass; fine for 55–550 Hz pitch).
      this.accum += chunk[i];
      this.accumN++;
      if (this.accumN < this.factor) continue;
      const x = this.accum / this.accumN;
      this.accum = 0;
      this.accumN = 0;
      this.onSample(x);
    }
  }

  private onSample(x: number): void {
    // Frame ring for pitch
    if (this.ringFill < this.frameLen) {
      this.ring[this.ringFill++] = x;
    } else {
      this.ring.copyWithin(0, 1);
      this.ring[this.frameLen - 1] = x;
    }
    this.sinceHop++;
    if (this.ringFill === this.frameLen && this.sinceHop >= this.hopLen) {
      this.sinceHop = 0;
      this.analyzeFrame();
    }
    // Energy envelope
    this.envAcc += x * x;
    this.envN++;
    if (this.envN >= this.envHop) {
      this.onEnvelope(Math.sqrt(this.envAcc / this.envN));
      this.envAcc = 0;
      this.envN = 0;
    }
  }

  private onEnvelope(e: number): void {
    // Track a slow noise floor and the running peak level.
    this.envFloor = e < this.envFloor ? e : this.envFloor * 0.999 + e * 0.001;
    this.envPeak = Math.max(this.envPeak * 0.9995, e);
    const gate = Math.max(this.envFloor * 3, this.envPeak * 0.1, 1e-3);
    if (e > gate) this.speechEnvFrames++;
    // Smooth (~40 ms time constant) so carrier ripple doesn't create peaks.
    const sm = this.envSmooth + 0.25 * (e - this.envSmooth);
    this.envSmooth = sm;
    this.framesSincePeak++;
    this.envTrough = Math.min(this.envTrough, sm);
    // Syllable nucleus: a local maximum of the smoothed envelope that rises
    // clearly (×1.6, ~4 dB) above the trough since the previous nucleus,
    // exceeds the speech gate, and is ≥ 120 ms after the previous one.
    const rising = sm > this.envPrev1;
    if (
      this.wasRising &&
      !rising &&
      this.envPrev1 > Math.max(gate, this.envTrough * 1.6) &&
      this.framesSincePeak >= 12
    ) {
      this.peaks++;
      this.framesSincePeak = 0;
      this.envTrough = this.envPrev1;
    }
    this.wasRising = rising;
    this.envPrev1 = sm;
  }

  private analyzeFrame(): void {
    this.totalFrames++;
    const x = this.ring;
    const n = x.length;
    let energy = 0;
    for (let i = 0; i < n; i++) energy += x[i] * x[i];
    if (energy / n < 1e-6) return; // silence

    // YIN difference function over lags for 55–550 Hz
    const tauMin = Math.max(2, Math.floor(this.rate / F0_MAX));
    const tauMax = Math.min(Math.floor(this.rate / F0_MIN), n - 2);
    const w = n - tauMax;
    let running = 0;
    let best = -1;
    let bestVal = 1;
    let prev = 1;
    for (let tau = 1; tau <= tauMax; tau++) {
      let d = 0;
      for (let j = 0; j < w; j++) {
        const diff = x[j] - x[j + tau];
        d += diff * diff;
      }
      running += d;
      const cmnd = running > 0 ? (d * tau) / running : 1;
      if (tau >= tauMin) {
        if (cmnd < YIN_THRESHOLD && cmnd < prev) {
          // descend to local minimum
          best = tau;
          bestVal = cmnd;
        } else if (best > 0 && cmnd > bestVal) {
          break;
        }
        if (best < 0 && cmnd < bestVal) {
          bestVal = cmnd;
        }
      }
      prev = cmnd;
    }
    if (best < 0 || bestVal >= YIN_THRESHOLD) return; // unvoiced
    const f0 = this.rate / best;
    if (f0 < F0_MIN || f0 > F0_MAX) return;
    const bin = Math.round(semis(f0) * BINS_PER_SEMITONE);
    if (bin < 0 || bin >= N_BINS) return;
    this.hist[bin] += 1;
    this.voicedFrames++;
    this.periodicitySum += 1 - bestVal;
  }

  private quantileHz(q: number): number {
    let total = 0;
    for (let i = 0; i < N_BINS; i++) total += this.hist[i];
    if (total === 0) return 0;
    let acc = 0;
    const target = q * total;
    for (let i = 0; i < N_BINS; i++) {
      acc += this.hist[i];
      if (acc >= target) return F0_MIN * 2 ** (i / BINS_PER_SEMITONE / 12);
    }
    return F0_MAX;
  }

  /** Current summary. Cheap; can be called repeatedly. */
  profile(): VoiceProfile {
    const voicedSeconds = this.voicedFrames * HOP_S;
    const speechSeconds = this.speechEnvFrames * ENV_HOP_S;
    const median = this.quantileHz(0.5);
    const iqr = median > 0 ? semis(this.quantileHz(0.75)) - semis(this.quantileHz(0.25)) : 0;
    const periodicity = this.voicedFrames ? this.periodicitySum / this.voicedFrames : 0;
    const amount = Math.min(1, voicedSeconds / this.fullConfidenceVoicedS);
    return {
      f0MedianHz: median,
      f0IqrSemitones: iqr,
      syllableRate: speechSeconds > 0.3 ? this.peaks / speechSeconds : 0,
      voicedSeconds,
      confidence: voicedSeconds < 0.5 ? 0 : amount * Math.min(1, periodicity / 0.8),
    };
  }

  /** Forget everything (revocation / session end). */
  reset(): void {
    this.hist.fill(0);
    this.voicedFrames = this.totalFrames = this.periodicitySum = 0;
    this.accum = this.accumN = this.ringFill = this.sinceHop = 0;
    this.ring.fill(0);
    this.envAcc = this.envN = this.envPrev1 = this.envSmooth = this.envPeak = 0;
    this.envTrough = Infinity;
    this.wasRising = false;
    this.envFloor = 1e-4;
    this.speechEnvFrames = this.peaks = 0;
    this.framesSincePeak = 1e9;
  }

  /** Bytes of state held, for the resource report. Independent of input length. */
  static stateBytes(sampleRate: number): number {
    const a = new VoiceAnalyzer(sampleRate);
    return a.hist.byteLength + a.ring.byteLength + 200;
  }
}
