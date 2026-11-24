import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createCeAdapter, presentCeText, CE_FINALIZE_TIMEOUT_MS, type WebSocketLike, type PcmCapture } from './ceSpeech';
import { RELEASE_TAIL_MS } from './config';
import { resolveFeatures } from '../engine/capabilities';
import { disclosureFor } from '../engine/privacy';
import type { AsrCallbacks, RecognizedSegment } from '../engine/ports';

vi.mock('./audio', () => ({ getMic: vi.fn(), setMicEnabled: vi.fn(), audioContext: vi.fn() }));

class FakeSocket implements WebSocketLike {
  binaryType = 'blob';
  readyState = 0;
  sent: (string | ArrayBufferLike)[] = [];
  closed = false;
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;
  send(d: string | ArrayBufferLike) {
    this.sent.push(d);
  }
  close() {
    this.closed = true;
  }
  open() {
    this.readyState = 1;
    this.onopen?.();
  }
  msg(m: object) {
    this.onmessage?.({ data: JSON.stringify(m) });
  }
  json() {
    return this.sent.filter((s): s is string => typeof s === 'string').map((s) => JSON.parse(s));
  }
  binary() {
    return this.sent.filter((s) => typeof s !== 'string') as ArrayBuffer[];
  }
}

function setup(mode: 'push-to-talk' | 'continuous' = 'push-to-talk', lang = 'en-US') {
  const sock = new FakeSocket();
  let onPcm: ((c: Float32Array) => void) | null = null;
  const capture: PcmCapture & { stopped: boolean } = { sampleRate: 48000, stopped: false, stop() { this.stopped = true; } };
  const finals: RecognizedSegment[] = [];
  const partials: string[] = [];
  const errors: [string, string, boolean][] = [];
  const activity: boolean[] = [];
  let ended = 0;
  const cb: AsrCallbacks = {
    onFinal: (s) => finals.push(s),
    onPartial: (t) => partials.push(t),
    onError: (c, m, f) => errors.push([c, m, f]),
    onSpeechActivity: (a) => activity.push(a),
    onEnd: () => ended++,
  };
  let t = 1_000;
  const asr = createCeAdapter(lang, mode, cb, {
    url: 'ws://bridge.test',
    openSocket: () => sock,
    startCapture: async (fn) => {
      onPcm = fn;
      return capture;
    },
    now: () => (t += 100),
  });
  return { asr, sock, capture, finals, partials, errors, activity, ended: () => ended, pcm: (n = 960) => onPcm?.(new Float32Array(n).fill(0.5)) };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('Communication Engine recognizer (ce-local)', () => {
  it('streams PCM after ready, maps partials and finals, and ends on the bridge end', async () => {
    const s = setup();
    await s.asr.start();
    expect(s.asr.engine).toBe('ce-local');
    expect(s.asr.route).toBe('ce-local');
    s.pcm(); // before the socket is open: held, not lost
    s.sock.open();
    expect(s.sock.json()[0]).toEqual({ type: 'start', lang: 'en-US', sampleRate: 48000, segmentation: 'continuous' });
    expect(s.sock.binary()).toHaveLength(0);
    s.sock.msg({ type: 'ready', adapter: 'ce.asr.sherpa-zipformer', model: 'm' });
    expect(s.sock.binary()).toHaveLength(1); // the held chunk is flushed
    s.pcm();
    const pcm = new Int16Array(s.sock.binary()[1]);
    expect(pcm.length).toBe(960);
    expect(pcm[0]).toBe(16383); // 0.5 × 32767, truncated by Int16Array
    s.sock.msg({ type: 'utterance', utterance: 0 });
    s.sock.msg({ type: 'partial', utterance: 0, revision: 0, committed: 'GOOD', provisional: 'MORNING COULD' });
    expect(s.partials.at(-1)).toBe('Good morning could');
    s.sock.msg({ type: 'final', utterance: 0, text: 'GOOD MORNING I AM HERE', reason: 'streamend', language: 'en', durationMs: 1500 });
    expect(s.finals).toHaveLength(1);
    expect(s.finals[0]).toMatchObject({ text: 'Good morning I am here', lang: 'en-US', engine: 'ce-local' });
    expect(s.finals[0].endedAt).toBeGreaterThan(s.finals[0].startedAt);
    // Release: tail, then a graceful stop; the session ends on "end".
    s.asr.stop();
    expect(s.sock.json().some((m) => m.type === 'stop')).toBe(false);
    s.pcm(); // still captured during the release tail
    vi.advanceTimersByTime(RELEASE_TAIL_MS);
    expect(s.capture.stopped).toBe(true);
    expect(s.sock.json().at(-1)).toEqual({ type: 'stop' });
    expect(s.asr.active).toBe(true);
    s.sock.msg({ type: 'end', aborted: false });
    expect(s.asr.active).toBe(false);
    expect(s.ended()).toBe(1);
    expect(s.sock.closed).toBe(true);
  });

  it('hands-free mode asks CE for activity segmentation and forwards activity', async () => {
    const s = setup('continuous');
    await s.asr.start();
    s.sock.open();
    expect(s.sock.json()[0].segmentation).toBe('activity');
    s.sock.msg({ type: 'ready', adapter: 'a', model: null });
    s.sock.msg({ type: 'activity', active: true });
    expect(s.activity.at(-1)).toBe(true);
    s.asr.stop(); // no tail in continuous mode
    expect(s.sock.json().at(-1)).toEqual({ type: 'stop' });
  });

  it('abort discards results and never reports them', async () => {
    const s = setup();
    await s.asr.start();
    s.sock.open();
    s.sock.msg({ type: 'ready', adapter: 'a', model: null });
    s.asr.abort();
    expect(s.sock.json().at(-1)).toEqual({ type: 'abort' });
    expect(s.asr.active).toBe(false);
    s.sock.msg({ type: 'final', utterance: 0, text: 'LATE', reason: 'stopped', language: null, durationMs: 100 });
    expect(s.finals).toHaveLength(0);
  });

  it('a missing bridge, an unsupported language and a stuck bridge end the push cleanly', async () => {
    const a = setup();
    await a.asr.start();
    a.sock.onerror?.();
    expect(a.errors[0][0]).toBe('network');
    expect(a.errors[0][1]).toContain('ws://bridge.test');
    expect(a.errors[0][2]).toBe(true);
    expect(a.asr.active).toBe(false);

    const b = setup('push-to-talk', 'fr-FR');
    await b.asr.start();
    b.sock.open();
    b.sock.msg({ type: 'error', code: 'language-unsupported', message: 'x', fatal: true });
    expect(b.errors[0][0]).toBe('language-unsupported');
    expect(b.asr.active).toBe(false);

    const c = setup();
    await c.asr.start();
    c.sock.open();
    c.sock.msg({ type: 'ready', adapter: 'a', model: null });
    c.asr.stop();
    vi.advanceTimersByTime(RELEASE_TAIL_MS + CE_FINALIZE_TIMEOUT_MS);
    expect(c.asr.active).toBe(false);
    expect(c.ended()).toBe(1);
  });

  it('presents model text in sentence case without changing words', () => {
    expect(presentCeText('  HOW MUCH DOES THIS COST  ')).toBe('How much does this cost');
    expect(presentCeText("I'M SURE I CAN")).toBe("I'm sure I can");
    expect(presentCeText('')).toBe('');
  });

  it('is selected only when built for CE, with its own consent route and disclosure', () => {
    const browser = { webSpeech: true, getUserMedia: true, mediaRecorder: true, webAudio: true, speechSynthesis: true, dataChannel: true, secureContext: true };
    expect(resolveFeatures(browser, null).stt).toBe('webspeech');
    const f = resolveFeatures({ ...browser, ceRecognizer: true }, null);
    expect(f).toMatchObject({ stt: 'ce-local', route: 'ce-local', canTalk: true, continuous: true });
    expect(disclosureFor('ce-local', null).points[0]).toContain('on this computer');
  });
});
