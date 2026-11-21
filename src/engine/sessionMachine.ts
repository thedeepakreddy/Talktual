// Explicit state machines for a call. Pure data + a tiny guard class, so the
// legal transitions are reviewable in one place and testable exhaustively.

import type { Activity, CallPhase } from './events';

export const PHASE_TRANSITIONS: Record<CallPhase, readonly CallPhase[]> = {
  idle: ['connecting', 'ended', 'failed'],
  connecting: ['waiting-for-peer', 'connected', 'reconnecting', 'ended', 'failed'],
  'waiting-for-peer': ['connected', 'reconnecting', 'ended', 'failed'],
  connected: ['waiting-for-peer', 'reconnecting', 'ended', 'failed'],
  reconnecting: ['connected', 'waiting-for-peer', 'ended', 'failed'],
  ended: [],
  failed: [],
};

export const ACTIVITY_TRANSITIONS: Record<Activity, readonly Activity[]> = {
  idle: ['permission-pending', 'ready', 'listening', 'stopped'],
  'permission-pending': ['ready', 'listening', 'stopped'],
  ready: ['permission-pending', 'listening', 'stopped', 'idle'],
  listening: ['processing', 'paused', 'stopped', 'ready'],
  processing: ['ready', 'listening', 'stopped'],
  paused: ['listening', 'ready', 'stopped'],
  stopped: ['idle', 'ready', 'permission-pending', 'listening'],
};

export const TERMINAL_PHASES: ReadonlySet<CallPhase> = new Set(['ended', 'failed']);

export class StateMachine<S extends string> {
  private current: S;

  constructor(
    initial: S,
    private readonly table: Record<S, readonly S[]>,
    private readonly onChange: (from: S, to: S, reason?: string) => void,
    private readonly onIllegal: (from: S, to: S) => void = () => {}
  ) {
    this.current = initial;
  }

  get state(): S {
    return this.current;
  }

  can(to: S): boolean {
    return this.table[this.current].includes(to);
  }

  /** Moves to `to` if legal. Same-state is a no-op. Illegal moves are reported and refused. */
  to(to: S, reason?: string): boolean {
    if (to === this.current) return true;
    if (!this.can(to)) {
      this.onIllegal(this.current, to);
      return false;
    }
    const from = this.current;
    this.current = to;
    this.onChange(from, to, reason);
    return true;
  }
}
