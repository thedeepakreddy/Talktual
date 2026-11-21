// Minimal typed event bus. A throwing listener is isolated (reported, not
// propagated) so one consumer can never break the engine or other consumers.

import type { EngineEvent, EngineEventOf, EngineEventType } from './events';

type Listener<E> = (event: E) => void;

export class EventBus {
  private byType = new Map<string, Set<Listener<EngineEvent>>>();
  private any = new Set<Listener<EngineEvent>>();

  constructor(private onListenerError: (err: unknown) => void = (err) => console.error('engine listener failed:', err)) {}

  on<T extends EngineEventType>(type: T, fn: Listener<EngineEventOf<T>>): () => void {
    let set = this.byType.get(type);
    if (!set) {
      set = new Set();
      this.byType.set(type, set);
    }
    set.add(fn as Listener<EngineEvent>);
    return () => set!.delete(fn as Listener<EngineEvent>);
  }

  onAny(fn: Listener<EngineEvent>): () => void {
    this.any.add(fn);
    return () => this.any.delete(fn);
  }

  emit(event: EngineEvent): void {
    for (const fn of [...(this.byType.get(event.type) ?? []), ...this.any]) {
      try {
        fn(event);
      } catch (err) {
        this.onListenerError(err);
      }
    }
  }

  listenerCount(): number {
    let n = this.any.size;
    for (const s of this.byType.values()) n += s.size;
    return n;
  }

  clear(): void {
    this.byType.clear();
    this.any.clear();
  }
}
