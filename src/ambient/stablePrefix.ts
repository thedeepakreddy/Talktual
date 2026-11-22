// Stable-prefix segmentation for an AMBIENT stream (Phase 8).
//
// A continuous recognizer reports a growing interim hypothesis for the current
// utterance and, eventually, a final. Waiting for every final adds seconds of
// delay when someone talks without pausing; translating every interim produces
// fragments, repetition and premature playback. This segmenter commits only
// what has STOPPED CHANGING:
//
//   * stable prefix — the longest word prefix shared by every interim seen in
//     the last `stableMs` (and by at least `minUpdates` interims);
//   * an early commit happens only when that stable, not-yet-committed run is
//     long enough to translate well (`minEarlyWords`), ending at a clause
//     boundary when one is available, or when the utterance has been open for
//     `maxOpenMs` (a long run-on);
//   * the final commits only the remainder after what was already committed.
//     If the recognizer revised words that were already committed, the
//     remainder is computed against the final and the unit is flagged
//     `revised` (committed text is never silently rewritten or repeated).
//
// Bounded: per-utterance history is capped; nothing is held after a final,
// reset() or end(). Fillers ("um", "ööö") and immediate duplicates are dropped.

import { collapseRepetitions, isFillerOnly } from '../conversation/repair';

export interface AmbientUnit {
  id: string;
  text: string;
  /** 'early' = committed from a stable interim; 'final' = from the recognizer's final. */
  kind: 'early' | 'final';
  /** ms epoch of the first interim/final of this utterance (approximate speech start as seen by the app). */
  heardAt: number;
  committedAt: number;
  /** The recognizer changed words that had already been committed earlier. */
  revised: boolean;
  /** Recognizer confidence was low (reported only by some engines). */
  uncertain: boolean;
}

export interface StablePrefixConfig {
  stableMs: number;
  minUpdates: number;
  minEarlyWords: number;
  maxOpenMs: number;
  /** An identical unit within this window is a duplicate (recognizer restarts can re-emit). */
  dupWindowMs: number;
  lowConfidence: number;
  maxHistory: number;
}

export const DEFAULT_STABLE_CONFIG: StablePrefixConfig = {
  stableMs: 700,
  minUpdates: 3,
  minEarlyWords: 6,
  maxOpenMs: 5000,
  dupWindowMs: 4000,
  lowConfidence: 0.45,
  maxHistory: 40,
};

const BOUNDARY = /[.!?;:,…]$/;
const norm = (w: string) => w.toLocaleLowerCase().replace(/[^\p{L}\p{N}']/gu, '');
const tokens = (s: string) => s.trim().split(/\s+/).filter(Boolean);

/** Number of leading words two token lists share (punctuation/case-insensitive). */
export function commonPrefix(a: string[], b: string[]): number {
  let i = 0;
  while (i < a.length && i < b.length && norm(a[i]) === norm(b[i])) i++;
  return i;
}

export class StablePrefixSegmenter {
  private history: { t: number; words: string[] }[] = [];
  private committed: string[] = [];
  private openedAt: number | null = null;
  private revisedPending = false;
  private recent: { t: number; key: string }[] = [];
  stats = { early: 0, final: 0, droppedFiller: 0, droppedDuplicate: 0, revised: 0 };

  constructor(
    private readonly lang: string,
    private readonly emit: (u: AmbientUnit) => void,
    private readonly now: () => number,
    private readonly cfg: StablePrefixConfig = DEFAULT_STABLE_CONFIG,
    private readonly newId: () => string = () => `a${Date.now().toString(36)}${(++idSeq).toString(36)}`
  ) {}

  /** An interim hypothesis for the current utterance. */
  partial(text: string): void {
    const t = this.now();
    const words = tokens(text);
    if (!words.length) return;
    if (this.openedAt === null) this.openedAt = t;
    // The recognizer revised already-committed words: remember for the next unit.
    if (this.committed.length && commonPrefix(words, this.committed) < this.committed.length) this.revisedPending = true;
    this.history.push({ t, words });
    if (this.history.length > this.cfg.maxHistory) this.history.shift();
    this.tryEarly(t);
  }

  /** The recognizer's final for the current utterance: commit what remains. */
  final(text: string, confidence?: number): void {
    const t = this.now();
    const words = tokens(text);
    const heardAt = this.openedAt ?? t;
    let start = 0;
    let revised = this.revisedPending;
    if (this.committed.length) {
      const shared = commonPrefix(words, this.committed);
      if (shared < this.committed.length) revised = true;
      start = shared;
    }
    const rest = words.slice(start).join(' ');
    this.resetUtterance();
    if (rest) this.commit(rest, 'final', heardAt, t, revised, confidence !== undefined && confidence > 0 && confidence < this.cfg.lowConfidence);
  }

  /** Recognizer ended or listening paused: forget the open utterance (interims are not commitments). */
  reset(): void {
    this.resetUtterance();
  }

  private resetUtterance(): void {
    this.history = [];
    this.committed = [];
    this.openedAt = null;
    this.revisedPending = false;
  }

  private tryEarly(t: number): void {
    // Words are stable if every hypothesis since `stableMs` ago — including the
    // one that was current at the start of that window — agrees on them.
    let first = -1;
    for (let i = this.history.length - 1; i >= 0; i--) {
      if (t - this.history[i].t >= this.cfg.stableMs) {
        first = i;
        break;
      }
    }
    if (first < 0) return; // not observed for long enough yet
    const window = this.history.slice(first);
    if (window.length < this.cfg.minUpdates) return;
    let stable = window[0].words.length;
    for (const h of window) stable = Math.min(stable, commonPrefix(window[0].words, h.words));
    const base = this.committed.length;
    if (stable <= base) return;
    const latest = window[window.length - 1].words;
    const run = latest.slice(base, stable);
    const longOpen = this.openedAt !== null && t - this.openedAt >= this.cfg.maxOpenMs;
    if (run.length < this.cfg.minEarlyWords && !(longOpen && run.length >= 3)) return;
    // Prefer ending at a clause boundary inside the stable run; never cut the last stable word mid-thought otherwise.
    let end = run.length;
    for (let i = run.length - 1; i >= Math.min(2, run.length - 1); i--) {
      if (BOUNDARY.test(run[i])) {
        end = i + 1;
        break;
      }
    }
    // Keep one stable word back unless forced: the next interim often attaches to it ("a" → "a table").
    if (end === run.length && !longOpen && run.length > this.cfg.minEarlyWords) end = run.length - 1;
    const chunk = run.slice(0, end);
    if (chunk.length < 3) return;
    const revised = this.revisedPending;
    this.revisedPending = false;
    this.committed = latest.slice(0, base + chunk.length);
    this.commit(chunk.join(' '), 'early', this.openedAt ?? t, t, revised, false);
    if (longOpen) this.openedAt = t; // the next run gets its own budget
  }

  private commit(raw: string, kind: AmbientUnit['kind'], heardAt: number, t: number, revised: boolean, uncertain: boolean): void {
    const clean = collapseRepetitions(raw, this.lang).text.trim();
    if (!clean || isFillerOnly(clean)) {
      this.stats.droppedFiller++;
      return;
    }
    const key = tokens(clean).map(norm).join(' ');
    this.recent = this.recent.filter((r) => t - r.t <= this.cfg.dupWindowMs);
    if (this.recent.some((r) => r.key === key)) {
      this.stats.droppedDuplicate++;
      return;
    }
    this.recent.push({ t, key });
    if (this.recent.length > 20) this.recent.shift();
    this.stats[kind]++;
    if (revised) this.stats.revised++;
    this.emit({ id: this.newId(), text: clean, kind, heardAt, committedAt: t, revised, uncertain });
  }
}

let idSeq = 0;
