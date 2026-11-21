// Fakes for CommunicationSession dependencies. Each records what the engine
// asked of it and exposes drivers so tests can play the other side.

import { vi } from 'vitest';
import type { CallHandlers, CallMessage } from '../lib/webrtc';
import type {
  AsrAdapter,
  AsrCallbacks,
  CapturePort,
  PlaybackItem,
  PlaybackListener,
  PlaybackPort,
  ServerCapabilities,
  StorageLike,
  TranslationRequest,
  TranslationResult,
} from '../engine/ports';
import type { BrowserCapabilities } from '../engine/capabilities';
import type { ListeningMode, PermissionState } from '../engine/events';
import { CommunicationSession, SessionConfig, SessionDeps } from '../engine/CommunicationSession';

export class FakeCall {
  handlers!: CallHandlers;
  sent: CallMessage[] = [];
  closed = 0;
  send = (m: CallMessage) => this.sent.push(m);
  close = () => {
    this.closed++;
  };
  status(s: string) {
    this.handlers.onStatus(s);
  }
  receive(m: CallMessage) {
    this.handlers.onMessage(m);
  }
  segments() {
    return this.sent.filter((m): m is Extract<CallMessage, { t: 'segment' }> => m.t === 'segment');
  }
}

export class FakeAsr implements AsrAdapter {
  static all: FakeAsr[] = [];
  active = false;
  starts = 0;
  stops = 0;
  aborts = 0;
  disposed = false;
  route: AsrAdapter['route'];
  /** Make start() fail like a denied permission prompt. */
  static denyNext = false;
  constructor(
    public engine: AsrAdapter['engine'],
    public lang: string,
    public mode: ListeningMode,
    public cb: AsrCallbacks
  ) {
    this.route = engine === 'webspeech' ? 'browser-speech' : 'server-stt';
    FakeAsr.all.push(this);
  }
  async start() {
    this.starts++;
    if (FakeAsr.denyNext) {
      FakeAsr.denyNext = false;
      this.cb.onError('permission-denied', 'denied', true);
      throw new Error('denied');
    }
    this.active = true;
  }
  stop() {
    this.stops++;
    // Graceful: the engine flushes asynchronously, then ends.
    queueMicrotask(() => {
      if (!this.active) return;
      this.active = false;
      this.cb.onEnd?.();
    });
  }
  abort() {
    this.aborts++;
    this.active = false;
  }
  dispose() {
    this.disposed = true;
  }
  // --- drivers ---
  say(text: string, isFinal = true) {
    if (!this.active) throw new Error('recognizer is not running — nothing can be heard');
    if (isFinal) this.cb.onFinal({ text, lang: this.lang, startedAt: 0, endedAt: 1, engine: this.engine });
    else this.cb.onPartial(text);
  }
}

export class FakePlayback implements PlaybackPort {
  queue: PlaybackItem[] = [];
  playingItem: PlaybackItem | null = null;
  played: PlaybackItem[] = [];
  cancelled = 0;
  private listeners = new Set<PlaybackListener>();
  get active() {
    return Boolean(this.playingItem) || this.queue.length > 0;
  }
  onStatus(fn: PlaybackListener) {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }
  private emit(id: string, s: Parameters<PlaybackListener>[1]) {
    for (const l of this.listeners) l(id, s, 'browser');
  }
  enqueue(item: PlaybackItem) {
    this.queue.push(item);
    this.emit(item.segmentId, 'queued');
    if (!this.playingItem) this.next();
  }
  private next() {
    const item = this.queue.shift();
    if (!item) return;
    this.playingItem = item;
    this.played.push(item);
    this.emit(item.segmentId, 'started');
  }
  /** Driver: the current segment finishes. */
  finish() {
    const cur = this.playingItem;
    if (!cur) return;
    this.playingItem = null;
    if (this.queue.length) this.next();
    this.emit(cur.segmentId, 'ended');
  }
  cancelAll() {
    const items = [...(this.playingItem ? [this.playingItem] : []), ...this.queue];
    this.playingItem = null;
    this.queue = [];
    this.cancelled += items.length;
    items.forEach((i) => this.emit(i.segmentId, 'cancelled'));
    return items.length;
  }
}

export class FakeCapture implements CapturePort {
  releases = 0;
  perm: PermissionState = 'prompt';
  private lost = new Set<() => void>();
  permission = async () => this.perm;
  release = () => {
    this.releases++;
  };
  onDeviceLost(fn: () => void) {
    this.lost.add(fn);
    return () => this.lost.delete(fn);
  }
  loseDevice() {
    this.lost.forEach((f) => f());
  }
}

export function memoryStorage(): StorageLike & { data: Map<string, string> } {
  const data = new Map<string, string>();
  return {
    data,
    getItem: (k) => data.get(k) ?? null,
    setItem: (k, v) => void data.set(k, v),
    removeItem: (k) => void data.delete(k),
  };
}

export const CHROME_LIKE: BrowserCapabilities = {
  webSpeech: true,
  getUserMedia: true,
  mediaRecorder: true,
  webAudio: true,
  speechSynthesis: true,
  dataChannel: true,
  secureContext: true,
};

export const FIREFOX_LIKE: BrowserCapabilities = { ...CHROME_LIKE, webSpeech: false };

export type TranslateImpl = (req: TranslationRequest) => Promise<TranslationResult>;

export const okTranslate: TranslateImpl = async (req) => ({
  segmentId: req.segmentId,
  status: req.from.split('-')[0] === req.to.split('-')[0] ? 'passthrough' : 'translated',
  text: req.from.split('-')[0] === req.to.split('-')[0] ? req.text : `[${req.to.split('-')[0]}] ${req.text}`,
  attempts: 1,
  latencyMs: 1,
});

export const failTranslate: TranslateImpl = async (req) => ({
  segmentId: req.segmentId,
  status: 'failed',
  text: req.text,
  error: { code: 'http', message: 'down' },
  attempts: 1,
  latencyMs: 1,
});

export interface Harness {
  session: CommunicationSession;
  call: FakeCall;
  playback: FakePlayback;
  capture: FakeCapture;
  storage: ReturnType<typeof memoryStorage>;
  asr: () => FakeAsr | undefined;
  translate: ReturnType<typeof vi.fn>;
  gesture: ReturnType<typeof vi.fn>;
  connects: number;
}

export function makeSession(
  opts: {
    config?: Partial<SessionConfig>;
    browser?: BrowserCapabilities;
    server?: Partial<ServerCapabilities>;
    translate?: TranslateImpl;
    consent?: boolean;
    deps?: Partial<SessionDeps>;
  } = {}
): Harness {
  FakeAsr.all = [];
  FakeAsr.denyNext = false;
  const call = new FakeCall();
  const playback = new FakePlayback();
  const capture = new FakeCapture();
  const storage = memoryStorage();
  if (opts.consent) {
    storage.setItem('talktual.consent.v1.browser-speech', 'granted');
    storage.setItem('talktual.consent.v1.server-stt', 'granted');
  }
  const translate = vi.fn(opts.translate ?? okTranslate);
  const gesture = vi.fn();
  const h = { connects: 0 } as Harness;
  const session = new CommunicationSession(
    { roomCode: 'ROOM01', lang: 'en-US', userName: 'Ana', ...opts.config },
    {
      connect: (_id, _url, handlers) => {
        h.connects++;
        call.handlers = handlers;
        return call;
      },
      signalingUrl: 'ws://test',
      createAsr: (lang, mode, cb) => new FakeAsr(opts.browser?.webSpeech === false ? 'server-stt' : 'webspeech', lang, mode, cb),
      translate,
      translateCaption: async (text, _f, to) => `[${to.split('-')[0]}] ${text}`,
      playback,
      capture,
      getServerCapabilities: async () => ({ tts: false, stt: true, translator: 'mymemory', liveCaptions: false, ...opts.server }),
      browser: opts.browser ?? CHROME_LIKE,
      storage,
      onUserGesture: gesture,
      ...opts.deps,
    }
  );
  return Object.assign(h, {
    session,
    call,
    playback,
    capture,
    storage,
    translate,
    gesture,
    asr: () => FakeAsr.all.at(-1),
  });
}

export const settle = async (n = 20) => {
  for (let i = 0; i < n; i++) await Promise.resolve();
};

/** start + peer joins + hello exchanged + caps loaded. */
export async function connected(h: Harness, peerLang = 'es-ES') {
  h.session.start();
  h.call.status('waiting-for-peer');
  h.call.status('connected');
  h.call.receive({ t: 'hello', lang: peerLang, name: 'Luis' });
  await settle();
}
