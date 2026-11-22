// Listener-side conversation policy: who holds the floor, which peer turns the
// local user interrupted, and what to do with speech that arrives while the
// local user is talking (overlap). Bounded and content-free: it stores turn
// ids, segment ids and timestamps only.

import { BoundedSet } from '../engine/segments';
import type { PlaybackItem } from '../engine/ports';

export interface HeldItem {
  item: PlaybackItem;
  turn?: string;
}

export class ListenerPolicy {
  private floor: { turn: string; open: boolean; at: number } | null = null;
  private interrupted: BoundedSet<string>;
  /** segmentId → turn, for segments currently queued/playing. */
  private active = new Map<string, string | undefined>();
  private held: HeldItem[] = [];

  constructor(
    private readonly maxTurns: number,
    private readonly maxHeld = 12,
    /** A floor that has seen no activity for this long is treated as closed (lost close message). */
    private readonly floorStaleMs = 8000
  ) {
    this.interrupted = new BoundedSet(maxTurns);
  }

  onPeerFloor(turn: string, open: boolean, now: number): void {
    this.floor = { turn, open, at: now };
  }

  /** Any sign the peer is still speaking in this turn (partial / provisional / segment). */
  touchPeerTurn(turn: string | undefined, now: number): void {
    if (turn && this.floor?.turn === turn) this.floor.at = now;
  }

  peerFloorOpen(now: number): boolean {
    return Boolean(this.floor?.open && now - this.floor.at < this.floorStaleMs);
  }

  peerTurn(): string | null {
    return this.floor?.turn ?? null;
  }

  isInterrupted(turn: string | undefined): boolean {
    return turn !== undefined && this.interrupted.has(turn);
  }

  noteActive(segmentId: string, turn: string | undefined): void {
    this.active.set(segmentId, turn);
    if (this.active.size > 64) this.active.delete(this.active.keys().next().value as string);
  }

  noteDone(segmentId: string): void {
    this.active.delete(segmentId);
  }

  /**
   * The local user starts talking. If the peer is still mid-turn, or their
   * speech is playing/queued, those turns are interrupted: later segments of
   * them are kept in the transcript but not spoken. Returns the turns marked.
   */
  bargeIn(now: number, playing: boolean): string[] {
    const turns = new Set<string>();
    if (this.peerFloorOpen(now) && this.floor) turns.add(this.floor.turn);
    if (playing) for (const t of this.active.values()) if (t) turns.add(t);
    for (const t of turns) this.interrupted.add(t);
    this.held = this.held.filter((h) => !h.turn || !turns.has(h.turn));
    return [...turns];
  }

  /** Hold an item while the local user is talking. Returns an item evicted by the bound, if any. */
  hold(h: HeldItem): HeldItem | null {
    this.held.push(h);
    return this.held.length > this.maxHeld ? this.held.shift()! : null;
  }

  get heldCount(): number {
    return this.held.length;
  }

  /** The local user stopped talking: release held items (still subject to their age bound). */
  releaseHeld(): HeldItem[] {
    const out = this.held.filter((h) => !this.isInterrupted(h.turn));
    this.held = [];
    return out;
  }

  /** The transport reconnected: floor messages may have been lost. */
  resetFloor(): void {
    this.floor = null;
  }

  clear(): void {
    this.floor = null;
    this.active.clear();
    this.held = [];
    this.interrupted = new BoundedSet(this.maxTurns);
  }
}
