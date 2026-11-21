// Two-party conversation simulator (Phase 4, M4.0/M4.9).
//
// Two REAL CommunicationSession instances (A and B) are connected through a
// simulated network with configurable latency and outages. Everything external
// is a deterministic fake driven by fake timers:
//   * ASR: scripted partial/final results (FakeAsr from the engine test fakes)
//   * translation: fixed latency, honours AbortSignal
//   * playback: duration proportional to text length; records start/end/cancel
// The harness only uses the session's public API, so the same scenarios run
// against the pre-Phase-4 engine (baseline) and the Phase 4 engine.

import { vi } from 'vitest';
import { CommunicationSession } from '../../src/engine/CommunicationSession';
import type { SessionDeps } from '../../src/engine/CommunicationSession';
import type { CallHandlers, CallMessage } from '../../src/lib/webrtc';
import type { PlaybackItem, PlaybackListener, PlaybackPort, TranslationRequest, TranslationResult } from '../../src/engine/ports';
import { FakeAsr, FakeCapture, memoryStorage, CHROME_LIKE } from '../../src/test/engineFakes';

export type Who = 'A' | 'B';

export interface SimOptions {
  /** One-way network latency, ms. */
  netMs?: number;
  /** Translation latency, ms. */
  translateMs?: number;
  /** Playback duration per character, ms (min 400 ms). */
  msPerChar?: number;
  /** Extra session deps (e.g. Phase 4 policy overrides). */
  deps?: Partial<SessionDeps>;
}

export interface PlayRecord {
  segmentId: string;
  text: string;
  queuedAt: number;
  startedAt?: number;
  endedAt?: number;
  cancelledAt?: number;
  dropped?: boolean;
}

/** Timed playback: the controller semantics Phase 2 tests rely on, plus timing. */
export class SimPlayback implements PlaybackPort {
  queue: PlaybackItem[] = [];
  current: PlaybackItem | null = null;
  log: PlayRecord[] = [];
  maxDepth = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private listeners = new Set<PlaybackListener>();
  constructor(private readonly msPerChar: number) {}
  get active() {
    return this.current !== null || this.queue.length > 0;
  }
  onStatus(fn: PlaybackListener) {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }
  private emit(id: string, s: Parameters<PlaybackListener>[1]) {
    for (const l of this.listeners) l(id, s, 'browser');
  }
  private rec(id: string) {
    return [...this.log].reverse().find((r) => r.segmentId === id)!;
  }
  enqueue(item: PlaybackItem) {
    this.queue.push(item);
    this.log.push({ segmentId: item.segmentId, text: item.text, queuedAt: Date.now() });
    this.maxDepth = Math.max(this.maxDepth, this.queue.length + (this.current ? 1 : 0));
    this.emit(item.segmentId, 'queued');
    if (!this.current) this.next();
  }
  private next() {
    const item = this.queue.shift();
    if (!item) return;
    // Optional staleness bound (Phase 4 adds `notAfter`; ignored if absent).
    const notAfter = (item as PlaybackItem & { notAfter?: number }).notAfter;
    if (notAfter !== undefined && Date.now() > notAfter) {
      this.rec(item.segmentId).dropped = true;
      this.emit(item.segmentId, 'dropped');
      this.next();
      return;
    }
    this.current = item;
    this.rec(item.segmentId).startedAt = Date.now();
    this.emit(item.segmentId, 'started');
    const dur = Math.max(400, item.text.length * this.msPerChar);
    this.timer = setTimeout(() => {
      this.timer = null;
      const cur = this.current!;
      this.current = null;
      this.rec(cur.segmentId).endedAt = Date.now();
      this.next();
      this.emit(cur.segmentId, 'ended');
    }, dur);
  }
  cancelAll() {
    const victims = [...(this.current ? [this.current] : []), ...this.queue];
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.current = null;
    this.queue = [];
    const now = Date.now();
    for (const v of victims) {
      this.rec(v.segmentId).cancelledAt = now;
      this.emit(v.segmentId, 'cancelled');
    }
    return victims.length;
  }
}

/** Fixed-latency translator that honours cancellation. */
export function simTranslator(ms: number, counter: { calls: number; aborted: number; completedAfterAbort: number }) {
  return (req: TranslationRequest): Promise<TranslationResult> =>
    new Promise((resolve) => {
      counter.calls++;
      const same = req.from.split('-')[0] === req.to.split('-')[0];
      const t = setTimeout(
        () => {
          if (req.signal?.aborted) counter.completedAfterAbort++;
          resolve({
            segmentId: req.segmentId,
            status: same ? 'passthrough' : 'translated',
            text: same ? req.text : `«${req.text}»`,
            attempts: 1,
            latencyMs: ms,
          });
        },
        same ? 0 : ms
      );
      req.signal?.addEventListener('abort', () => {
        clearTimeout(t);
        counter.aborted++;
        resolve({ segmentId: req.segmentId, status: 'cancelled', text: req.text, attempts: 1, latencyMs: 0 });
      });
    });
}

export class SimNet {
  up = true;
  delivered = 0;
  dropped = 0;
  /** Messages generated while the link is down, re-sent on recovery (like the transport outbox). */
  private held: Array<() => void> = [];
  constructor(private readonly latency: number) {}
  send(fn: () => void, isSegment: boolean) {
    if (!this.up) {
      if (isSegment) this.held.push(fn);
      else this.dropped++;
      return;
    }
    setTimeout(() => {
      this.delivered++;
      fn();
    }, this.latency);
  }
  setUp(up: boolean) {
    this.up = up;
    if (up) {
      const h = this.held;
      this.held = [];
      h.forEach((fn) => this.send(fn, true));
    }
  }
}

export interface Party {
  who: Who;
  session: CommunicationSession;
  playback: SimPlayback;
  handlers: CallHandlers;
  sent: CallMessage[];
  /** Send times (ms since sim start), parallel to `sent`. */
  sentAt: number[];
  asr: () => FakeAsr;
  closed: number;
}

export interface Sim {
  A: Party;
  B: Party;
  net: SimNet;
  translations: { calls: number; aborted: number; completedAfterAbort: number };
  t0: number;
  /** First time each (normalized) text was finalized by a recognizer, ms since start. */
  said: Map<string, number>;
}

export function createSim(opts: SimOptions = {}): Sim {
  const net = new SimNet(opts.netMs ?? 60);
  const t0 = Date.now();
  const translations = { calls: 0, aborted: 0, completedAfterAbort: 0 };
  const parties = {} as Record<Who, Party>;
  for (const who of ['A', 'B'] as Who[]) {
    const other: Who = who === 'A' ? 'B' : 'A';
    const playback = new SimPlayback(opts.msPerChar ?? 45);
    const storage = memoryStorage();
    storage.setItem('talktual.consent.v1.browser-speech', 'granted');
    const party = { who, playback, sent: [] as CallMessage[], sentAt: [] as number[], closed: 0 } as unknown as Party;
    const asrs: FakeAsr[] = [];
    party.asr = () => asrs.at(-1)!;
    party.session = new CommunicationSession(
      { roomCode: 'SIM001', lang: who === 'A' ? 'en-US' : 'es-ES', userName: who },
      {
        connect: (_id, _url, handlers) => {
          party.handlers = handlers;
          return {
            send: (m: CallMessage) => {
              party.sent.push(m);
              party.sentAt.push(Date.now() - t0);
              net.send(() => parties[other].handlers.onMessage(m), m.t === 'segment');
            },
            close: () => {
              party.closed++;
            },
          };
        },
        signalingUrl: 'sim://',
        createAsr: (lang, mode, cb) => {
          const a = new FakeAsr('webspeech', lang, mode, cb);
          asrs.push(a);
          return a;
        },
        translate: simTranslator(opts.translateMs ?? 300, translations),
        translateCaption: async (text) => text,
        playback,
        capture: new FakeCapture(),
        getServerCapabilities: async () => ({ tts: false, stt: false, translator: 'sim', liveCaptions: false }),
        browser: CHROME_LIKE,
        storage,
        ...opts.deps,
      }
    );
    parties[who] = party;
  }
  return { A: parties.A, B: parties.B, net, translations, t0, said: new Map() };
}

/** Start both sessions and connect them (hello exchange through the network). */
export async function connectSim(sim: Sim) {
  for (const p of [sim.A, sim.B]) {
    p.session.start();
    p.handlers.onStatus('waiting-for-peer');
    p.handlers.onStatus('connected');
  }
  await vi.advanceTimersByTimeAsync(500);
}

export async function at(sim: Sim, ms: number) {
  const now = Date.now() - sim.t0;
  if (ms > now) await vi.advanceTimersByTimeAsync(ms - now);
}
