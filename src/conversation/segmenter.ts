// Turn/pause detection + adaptive segmentation for the LOCAL speaker.
//
// Input: recognizer finals/partials and explicit cues (push-to-talk release,
// recognizer end). Output: committed units (what gets translated and sent),
// plus hold/revise notifications for provisional output.
//
// Policy (all bounded):
//   * A final that looks complete commits immediately (no added delay).
//   * A final that looks unfinished ("…book a", "i think that") is HELD as a
//     pause candidate for an adaptive window (min..max hold). Silence alone does
//     not end the thought; continued speech (partials) extends the hold, but
//     never beyond maxOpenMs from when the unit opened.
//   * The next final either continues the held fragment (merge) or restarts it
//     (replace, same id, new revision) — see repair.resolveRestart.
//   * Hesitation sounds and low-confidence 1–2 word results are dropped.
//   * Push-to-talk release / recognizer end are end-of-turn cues: whatever is
//     held commits (after at most releaseFlushMs for the recognizer's flush).
// Holds only the CURRENT open unit; committed text is never revised.

import { AdaptiveHold } from './pauseModel';
import { collapseRepetitions, isFillerOnly, looksIncomplete, resolveRestart } from './repair';
import { words } from './lexicon';
import { canTransition, CommitReason, ConversationConfig, UnitState } from './types';

export interface Unit {
  id: string;
  turn: string;
  text: string;
  /** Revision of a held unit (0 = first version). */
  rev: number;
  /** Self-repair changed the recognized text. */
  repaired: boolean;
  /** Recognizer confidence was low for part of this unit. */
  uncertain: boolean;
}

export interface SegmenterEvents {
  /** The unit is held as a pause candidate (not yet final). */
  onHold(unit: Unit): void;
  /** A held unit was superseded by a restart (same id, rev+1); still held. */
  onRevise(unit: Unit): void;
  /** Final: translate and send. */
  onCommit(unit: Unit, reason: CommitReason): void;
  /** A held unit was thrown away (capture aborted). */
  onDiscard(unit: Unit): void;
  /** A result was dropped as a hesitation sound or noise. */
  onDrop(kind: 'filler' | 'noise'): void;
  /** Bookkeeping: an illegal state transition was attempted (should never happen). */
  onIllegal?(from: UnitState, to: UnitState): void;
}

export interface Clock {
  now(): number;
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(t: unknown): void;
}

interface Open {
  unit: Unit;
  state: UnitState;
  openedAt: number;
  heldAt: number;
}

export class Segmenter {
  private open: Open | null = null;
  private timer: unknown = null;
  private released = false;
  private turn = '';
  private readonly hold: AdaptiveHold;

  constructor(
    private lang: string,
    private readonly cfg: ConversationConfig,
    private readonly ev: SegmenterEvents,
    private readonly clock: Clock,
    private readonly newId: () => string
  ) {
    this.hold = new AdaptiveHold(cfg.minHoldMs, cfg.maxHoldMs, cfg.defaultHoldMs);
  }

  /** Current hold window (ms) — exposed for tests/diagnostics. */
  get holdMs(): number {
    return this.hold.holdMs();
  }

  get current(): { unit: Unit; state: UnitState } | null {
    return this.open ? { unit: { ...this.open.unit }, state: this.open.state } : null;
  }

  setLang(lang: string): void {
    this.lang = lang;
  }

  /** A new speaking turn begins (press-to-talk, or floor reopened). */
  beginTurn(turn: string): void {
    if (this.open) this.commit('end-of-input');
    this.turn = turn;
    this.released = false;
  }

  get turnId(): string {
    return this.turn;
  }

  partial(text: string): void {
    // Continued speech while a fragment is held: the thought is still going.
    if (!text.trim() || !this.open || this.released) return;
    if (this.open.state === 'pause-candidate' || this.open.state === 'provisional') this.schedule(this.hold.holdMs());
  }

  final(raw: string, confidence?: number): void {
    const { text, repaired } = collapseRepetitions(raw, this.lang);
    if (isFillerOnly(text)) {
      this.ev.onDrop('filler');
      return;
    }
    // Browsers report 0 when they don't know: treat 0/undefined as "unknown".
    const known = typeof confidence === 'number' && confidence > 0;
    if (known && confidence! < this.cfg.noiseBelow && words(text).length <= 2) {
      this.ev.onDrop('noise');
      return;
    }
    const uncertain = known && confidence! < this.cfg.uncertainBelow;
    const now = this.clock.now();

    if (this.open) {
      this.hold.observeGap(now - this.open.heldAt);
      const r = resolveRestart(this.open.unit.text, text, this.lang);
      if (r.kind === 'replace') {
        const u = this.open.unit;
        u.text = collapseRepetitions(r.text, this.lang).text;
        u.rev++;
        u.repaired = true;
        u.uncertain = uncertain;
        this.ev.onRevise({ ...u });
      } else if (this.open.unit.text.length + 1 + text.length > this.cfg.maxUnitChars) {
        this.commit('max-length');
        this.openUnit(text, repaired, uncertain, now);
      } else {
        const u = this.open.unit;
        u.text = `${u.text} ${text}`;
        u.repaired ||= repaired;
        u.uncertain ||= uncertain;
      }
    } else {
      this.openUnit(text, repaired, uncertain, now);
    }
    this.evaluate(now);
  }

  /** Push-to-talk released: the turn is ending; commit once the recognizer flushes. */
  release(): void {
    this.released = true;
    if (this.open) this.schedule(this.cfg.releaseFlushMs, 'release');
  }

  /** The recognizer stopped: nothing more will arrive for this turn. */
  endOfInput(): void {
    if (this.open) this.commit(this.released ? 'release' : 'end-of-input');
    this.released = false;
  }

  /** Capture aborted (mic off, mode/language change, leave): discard the held unit. */
  cancel(): Unit | null {
    this.clear();
    const o = this.open;
    this.open = null;
    this.released = false;
    if (!o) return null;
    this.move(o, 'superseded');
    this.ev.onDiscard({ ...o.unit });
    return o.unit;
  }

  /** Drops all state including learned timing (session end). */
  dispose(): void {
    this.cancel();
    this.hold.reset();
    this.turn = '';
  }

  // ---------------------------------------------------------------------------

  private openUnit(text: string, repaired: boolean, uncertain: boolean, now: number): void {
    this.open = {
      unit: { id: this.newId(), turn: this.turn, text, rev: 0, repaired, uncertain },
      state: 'speech-active',
      openedAt: now,
      heldAt: now,
    };
  }

  private evaluate(now: number): void {
    const o = this.open!;
    if (o.unit.text.length >= this.cfg.maxUnitChars) return this.commit('max-length');
    if (this.released) return this.commit('release');
    if (now - o.openedAt >= this.cfg.maxOpenMs) return this.commit('max-open');
    if (!looksIncomplete(o.unit.text, this.lang)) return this.commit('complete');
    o.heldAt = now;
    if (o.state !== 'provisional') this.move(o, 'pause-candidate');
    this.schedule(this.hold.holdMs());
    this.ev.onHold({ ...o.unit });
  }

  /** The session sent a provisional translation for the held unit. */
  markProvisional(id: string): void {
    if (this.open?.unit.id === id) this.move(this.open, 'provisional');
  }

  private schedule(ms: number, reason: CommitReason = 'hold-expired'): void {
    const o = this.open;
    if (!o) return;
    this.clear();
    const now = this.clock.now();
    const deadline = Math.min(now + ms, o.openedAt + this.cfg.maxOpenMs);
    const why: CommitReason = reason === 'release' ? 'release' : now + ms > o.openedAt + this.cfg.maxOpenMs ? 'max-open' : 'hold-expired';
    this.timer = this.clock.setTimeout(() => {
      this.timer = null;
      if (this.open === o) this.commit(why);
    }, Math.max(0, deadline - now));
  }

  private commit(reason: CommitReason): void {
    this.clear();
    const o = this.open;
    if (!o) return;
    this.open = null;
    this.move(o, 'turn-candidate');
    this.move(o, 'final');
    this.ev.onCommit({ ...o.unit }, reason);
  }

  private move(o: Open, to: UnitState): void {
    if (o.state === to && to !== 'pause-candidate' && to !== 'provisional') return;
    if (!canTransition(o.state, to)) {
      this.ev.onIllegal?.(o.state, to);
      return;
    }
    o.state = to;
  }

  private clear(): void {
    if (this.timer !== null) this.clock.clearTimeout(this.timer);
    this.timer = null;
  }
}
