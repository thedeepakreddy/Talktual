// Network state for the processing policy: browser online/offline events plus
// provider reachability (consecutive network failures). Bounded, no polling of
// third parties: recovery is detected from the next successful call or the
// browser's 'online' event, after a cooldown.

import type { NetworkState } from './policy';

export interface NetworkSource {
  onLine(): boolean;
  subscribe(fn: () => void): () => void;
}

export function browserNetworkSource(win: (Window & typeof globalThis) | undefined): NetworkSource {
  return {
    onLine: () => win?.navigator?.onLine ?? true,
    subscribe: (fn) => {
      if (!win) return () => {};
      win.addEventListener('online', fn);
      win.addEventListener('offline', fn);
      return () => {
        win.removeEventListener('online', fn);
        win.removeEventListener('offline', fn);
      };
    },
  };
}

export class NetworkMonitor {
  private failures = 0;
  private unreachableSince: number | null = null;
  private unsub: () => void;

  constructor(
    private readonly source: NetworkSource,
    private readonly now: () => number,
    private readonly onState: (s: NetworkState) => void,
    private readonly threshold = 2,
    /** After this long, cloud is tried again even without a success signal. */
    private readonly cooldownMs = 30_000
  ) {
    this.unsub = source.subscribe(() => this.emit());
  }

  get state(): NetworkState {
    if (!this.source.onLine()) return 'offline';
    if (this.unreachableSince !== null && this.now() - this.unreachableSince < this.cooldownMs) return 'providers-unreachable';
    return 'online';
  }

  /** A provider call failed for a network reason (not a provider error). */
  networkFailure(): void {
    this.failures++;
    if (this.failures >= this.threshold && this.unreachableSince === null) this.unreachableSince = this.now();
    this.emit();
  }

  success(): void {
    this.failures = 0;
    this.unreachableSince = null;
    this.emit();
  }

  /** Re-evaluate (e.g. after the cooldown, or on transport reconnect). */
  refresh(): void {
    if (this.unreachableSince !== null && this.now() - this.unreachableSince >= this.cooldownMs) {
      this.unreachableSince = null;
      this.failures = 0;
    }
    this.emit();
  }

  private emit(): void {
    this.onState(this.state);
  }

  dispose(): void {
    this.unsub();
  }
}
