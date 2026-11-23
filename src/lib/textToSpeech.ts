// Text-to-speech playback.
//
// Two things matter here. First, audio has to start before the whole clip is
// generated — the server streams raw PCM and we schedule each chunk into the
// AudioContext as it lands, so the peer hears the first syllable in a couple
// hundred milliseconds instead of waiting on a complete MP3. Second, segments
// have to play strictly in order, because a long utterance arrives as several
// of them and playing them concurrently would be gibberish.

import { API_BASE } from "./config";
import { audioContext } from "./audio";

import type { PlaybackItem, PlaybackListener, PlaybackPort, TtsRoute } from "../engine/ports";
import type { RenderHints } from "../voicemirror/types";
import type { PlaybackStatus } from "../engine/events";

/** Pending (not yet started) segments kept before the oldest is dropped. */
export const PLAYBACK_QUEUE_MAX = 12;
/** How far ahead of the audio clock streamed PCM may be scheduled. */
export const PCM_MAX_AHEAD_S = 10;

interface PlaybackState {
  aborted: boolean;
  /** Cancels the in-flight TTS request on barge-in so we stop downloading audio nobody will hear. */
  controller: AbortController;
  sources: AudioBufferSourceNode[];
  gain: GainNode | null;
  element: HTMLAudioElement | null;
  /** Phase 8: output level 0..1 for this item (default 1). */
  volume?: number;
}

type Engine = "server" | "browser";

interface QueueItem extends PlaybackItem {
  hooks?: { onStart?: () => void; onEnd?: () => void };
  done: () => void;
}

/**
 * Plays segments strictly one after another, in arrival order.
 *
 * Bounded: at most PLAYBACK_QUEUE_MAX segments wait; beyond that the oldest
 * waiting one is dropped (status `dropped`) — a listener can't usefully catch
 * up on a long backlog, and an unbounded queue grows without limit when the
 * peer talks faster than speech can be played.
 *
 * Cancellable: cancelAll() stops the playing segment, aborts its network
 * request and drops everything waiting (status `cancelled`). Nothing queued
 * before the cancel can start afterwards.
 */
export class PlaybackController implements PlaybackPort {
  private queue: QueueItem[] = [];
  private playing: QueueItem | null = null;
  private current: PlaybackState | null = null;
  private listeners = new Set<PlaybackListener>();
  private pumping = false;

  constructor(private readonly maxQueue = PLAYBACK_QUEUE_MAX) {}

  get active(): boolean {
    return this.playing !== null || this.queue.length > 0;
  }

  get depth(): number {
    return this.queue.length + (this.playing ? 1 : 0);
  }

  onStatus(fn: PlaybackListener): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private emit(id: string, status: PlaybackStatus, engine?: Engine, failure?: PlaybackFailure) {
    for (const l of this.listeners) {
      try {
        l(id, status, engine, failure);
      } catch (err) {
        console.error("playback listener failed:", err);
      }
    }
  }

  enqueue(item: PlaybackItem, hooks?: QueueItem["hooks"]): Promise<void> {
    const text = item.text.trim();
    if (!text) return Promise.resolve();
    return new Promise<void>((resolve) => {
      this.queue.push({ ...item, text, hooks, done: resolve });
      this.emit(item.segmentId, "queued");
      while (this.queue.length > this.maxQueue) {
        const dropped = this.queue.shift()!;
        this.emit(dropped.segmentId, "dropped");
        dropped.hooks?.onEnd?.();
        dropped.done();
      }
      void this.pump();
    });
  }

  cancelAll(): number {
    const victims = [...(this.playing ? [this.playing] : []), ...this.queue];
    this.queue = [];
    const cur = this.current;
    if (cur) {
      cur.aborted = true;
      cur.controller.abort();
      cur.sources.forEach((s) => {
        try {
          s.stop();
        } catch {
          /* already finished */
        }
      });
      try {
        cur.gain?.disconnect();
      } catch {
        /* ignore */
      }
      if (cur.element) {
        cur.element.pause();
        cur.element.src = "";
      }
      this.current = null;
    }
    if (typeof window !== "undefined" && "speechSynthesis" in window) {
      try {
        lastCancelAt = Date.now();
        window.speechSynthesis.cancel();
      } catch {
        /* ignore */
      }
    }
    const playingItem = this.playing;
    this.playing = null;
    for (const v of victims) {
      this.emit(v.segmentId, "cancelled");
      // The playing item's own completion path will call its hooks.
      if (v !== playingItem) {
        v.hooks?.onEnd?.();
        v.done();
      }
    }
    return victims.length;
  }

  private async pump(): Promise<void> {
    if (this.pumping) return;
    this.pumping = true;
    try {
      while (!this.playing && this.queue.length) {
        const item = this.queue.shift()!;
        // Phase 4: speech that waited past its age bound is stale — drop it
        // rather than play it late.
        if (item.notAfter !== undefined && Date.now() > item.notAfter) {
          this.emit(item.segmentId, "dropped");
          item.hooks?.onEnd?.();
          item.done();
          continue;
        }
        this.playing = item;
        const state: PlaybackState = {
          aborted: false,
          controller: new AbortController(),
          sources: [],
          gain: null,
          element: null,
          volume: clampVolume(item.volume),
        };
        this.current = state;
        item.hooks?.onStart?.();
        this.emit(item.segmentId, "started");
        let outcome: Outcome = { engine: "none", failure: "error" };
        try {
          outcome = await playOne(item.text, item.lang, state, safeHints(item.voice), safeRoute(item.ttsRoute));
        } catch (err) {
          console.error("TTS playback failed:", err instanceof Error ? err.message : "error");
          outcome = state.aborted ? { engine: "cancelled" } : { engine: "none", failure: "error" };
        }
        if (this.current === state) this.current = null;
        const wasCancelled = state.aborted || this.playing !== item;
        if (this.playing === item) this.playing = null;
        if (!wasCancelled) {
          if (outcome.engine === "none") this.emit(item.segmentId, "failed", undefined, outcome.failure ?? "error");
          else this.emit(item.segmentId, "ended", outcome.engine === "cancelled" ? undefined : outcome.engine);
        }
        item.hooks?.onEnd?.();
        item.done();
      }
    } finally {
      this.pumping = false;
    }
  }
}

function clampVolume(v: number | undefined): number | undefined {
  if (v === undefined || !Number.isFinite(v)) return undefined;
  return Math.min(1, Math.max(0, v));
}

// --- Phase 1 API, kept for existing callers --------------------------------------

let defaultController = new PlaybackController();
let seq = 0;

/** The shared controller the app uses. */
export function playbackController(): PlaybackController {
  return defaultController;
}

/** Test hook: start from a clean controller. */
export function resetPlaybackForTests() {
  defaultController = new PlaybackController();
  resetTtsStateForTests();
}

export function isSpeaking(): boolean {
  return defaultController.active;
}

/**
 * Queues one segment. Segments play back to back in call order; the returned
 * promise settles when this segment finishes (or is cut off / dropped).
 */
export function enqueueSpeech(
  text: string,
  lang: string,
  hooks?: { onStart?: () => void; onEnd?: () => void }
): Promise<void> {
  return defaultController.enqueue({ segmentId: `tts-${seq++}`, text, lang }, hooks);
}

/** Barge-in: kill whatever is playing and drop the rest of the queue. */
export function stopSpeaking() {
  defaultController.cancelAll();
}

/** Which engine actually produced audio, "none" if nothing could, "cancelled" if cut off. */
function safeHints(resolve?: () => RenderHints | undefined): RenderHints | undefined {
  try {
    return resolve?.();
  } catch {
    return undefined; // a hint failure must never block playback
  }
}

/** How long to wait for a suspended AudioContext to resume before using the browser voice. */
export const AUDIO_RESUME_WAIT_MS = 250;

/** True if the shared AudioContext is (or within `waitMs` becomes) running. Never throws. */
async function audioContextRunning(waitMs: number): Promise<boolean> {
  let ac: AudioContext;
  try {
    ac = audioContext();
  } catch {
    return false;
  }
  if (ac.state === "running") return true;
  // resume() without a user gesture can stay pending forever: bound the wait.
  await Promise.race([ac.resume().catch(() => {}), new Promise((r) => setTimeout(r, waitMs))]);
  return (ac.state as AudioContextState) === "running";
}

function safeRoute(resolve?: () => TtsRoute): TtsRoute {
  try {
    return resolve?.() ?? "cloud-first";
  } catch {
    return "device-only"; // fail closed: never send text to the server on a policy error
  }
}

/** Why a segment could not be spoken (passed with a `failed` status). */
export type { PlaybackFailure } from "../engine/ports";
import type { PlaybackFailure } from "../engine/ports";

/** Server TTS: give up on the request (→ browser voice) if no response arrives in this long. */
export const SERVER_TTS_FIRST_BYTE_MS = 4000;
/** After the server says it has no voice configured (or isn't there), stop asking for a while. */
const SERVER_UNCONFIGURED_BACKOFF_MS = 5 * 60_000;
/** After a transient server failure (timeout, 5xx, network), skip it briefly. */
const SERVER_FAILURE_BACKOFF_MS = 20_000;
let serverTtsSkipUntil = 0;

interface Outcome {
  engine: Engine | "none" | "cancelled";
  failure?: PlaybackFailure;
}

async function playOne(
  text: string,
  lang: string,
  state: PlaybackState,
  hints?: RenderHints,
  route: TtsRoute = "cloud-first"
): Promise<Outcome> {
  // Phase 6: on-device voices first (or only). Never a network voice here.
  if (route !== "cloud-first") {
    const r = await browserSpeak(text, lang, state, hints, true);
    if (state.aborted) return { engine: "cancelled" };
    if (r === "ok") return { engine: "browser" };
    if (route === "device-only") return { engine: "none", failure: r };
  }
  // Known-unavailable server: go straight to the browser voice instead of
  // paying a round trip per segment.
  if (Date.now() < serverTtsSkipUntil) return speakInBrowser(text, lang, state, hints);

  const req = new AbortController();
  const forward = () => req.abort();
  state.controller.signal.addEventListener("abort", forward);
  let timedOut = false;
  const timer = setTimeout(() => {
    timedOut = true;
    req.abort();
  }, SERVER_TTS_FIRST_BYTE_MS);
  try {
    let res: Response;
    try {
      res = await fetch(`${API_BASE}/api/tts`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        // Only coarse, non-identifying hints go to the server; it picks the voice.
        body: JSON.stringify(hints ? { text, lang, voice: { register: hints.register, speed: hints.ttsSpeed } } : { text, lang }),
        signal: req.signal,
      });
    } catch (err) {
      if (!state.aborted) serverTtsSkipUntil = Date.now() + SERVER_FAILURE_BACKOFF_MS;
      throw timedOut ? new Error("TTS server did not answer in time") : err;
    } finally {
      clearTimeout(timer);
    }

    if (!res.ok || !res.body) {
      // 503 = no voice configured; 404/405/501 = no TTS endpoint on this server.
      const unconfigured = [404, 405, 501, 503].includes(res.status);
      serverTtsSkipUntil = Date.now() + (unconfigured ? SERVER_UNCONFIGURED_BACKOFF_MS : SERVER_FAILURE_BACKOFF_MS);
      res.body?.cancel().catch(() => {});
      throw new Error(`TTS request failed: ${res.status}`);
    }

    const format = res.headers.get("X-Audio-Format") || "";
    if (format === "pcm") {
      // Reliability fix: streamed PCM plays through the AudioContext. If the
      // browser keeps it suspended (no user gesture yet, tab backgrounded, iOS
      // interruption), scheduled audio is SILENT and its clock never advances.
      // Don't play into it: fall back to the browser voice (catch below).
      if (!(await audioContextRunning(AUDIO_RESUME_WAIT_MS))) {
        res.body?.cancel().catch(() => {});
        throw new Error("audio context not running");
      }
      const rate = Number(res.headers.get("X-Sample-Rate")) || 24000;
      await playPcmStream(res, rate, state);
    } else {
      await playEncoded(res, state);
    }
    return { engine: state.aborted ? "cancelled" : "server" };
  } catch (err) {
    if (state.aborted) return { engine: "cancelled" };
    console.warn("Falling back to browser speech synthesis:", err instanceof Error ? err.message : "error");
    return speakInBrowser(text, lang, state, hints);
  } finally {
    state.controller.signal.removeEventListener("abort", forward);
  }
}

async function speakInBrowser(text: string, lang: string, state: PlaybackState, hints?: RenderHints): Promise<Outcome> {
  const r = await browserSpeak(text, lang, state, hints);
  if (state.aborted) return { engine: "cancelled" };
  return r === "ok" ? { engine: "browser" } : { engine: "none", failure: r };
}

/**
 * Schedules raw signed-16-bit little-endian PCM onto the audio clock as it
 * streams in. Each chunk is queued to start exactly where the previous one
 * ended, which keeps playback gapless even though the chunks arrive unevenly.
 */
async function playPcmStream(res: Response, sampleRate: number, state: PlaybackState): Promise<void> {
  const ac = audioContext();
  if (ac.state === "suspended") await ac.resume().catch(() => {});

  const gain = ac.createGain();
  if (state.volume !== undefined && gain.gain) gain.gain.value = state.volume;
  gain.connect(ac.destination);
  state.gain = gain;

  const reader = res.body!.getReader();
  // A small lead-in absorbs jitter in the first few chunks without being
  // audible as delay.
  const LEAD_SECONDS = 0.06;
  let playhead = 0;
  let leftoverByte: number | null = null;
  let lastSource: AudioBufferSourceNode | null = null;

  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done || state.aborted || !value) break;

      let bytes: Uint8Array = value;

      // PCM samples are two bytes; a chunk boundary can split one in half.
      if (leftoverByte !== null) {
        const merged = new Uint8Array(bytes.length + 1);
        merged[0] = leftoverByte;
        merged.set(bytes, 1);
        bytes = merged;
        leftoverByte = null;
      }
      if (bytes.length % 2 === 1) {
        leftoverByte = bytes[bytes.length - 1];
        bytes = bytes.subarray(0, bytes.length - 1);
      }
      if (bytes.length === 0) continue;

      // Copy into a fresh, aligned buffer — Int16Array views require the
      // underlying offset to be even, which a subarray can't guarantee.
      const aligned = new ArrayBuffer(bytes.length);
      new Uint8Array(aligned).set(bytes);
      const samples = new Int16Array(aligned);

      const buffer = ac.createBuffer(1, samples.length, sampleRate);
      const channel = buffer.getChannelData(0);
      for (let i = 0; i < samples.length; i++) channel[i] = samples[i] / 32768;

      const source = ac.createBufferSource();
      source.buffer = buffer;
      source.connect(gain);

      // A context that stopped running mid-stream would never advance its
      // clock: stop scheduling (caller falls back to the browser voice).
      if ((ac.state as AudioContextState) !== "running") throw new Error("audio context not running");
      // Backpressure: don't schedule more than PCM_MAX_AHEAD_S into the future.
      // The reader (and so the network stream) waits instead of buffering
      // the whole clip in audio nodes.
      while (!state.aborted && playhead - ac.currentTime > PCM_MAX_AHEAD_S) {
        if ((ac.state as AudioContextState) !== "running") throw new Error("audio context not running");
        await new Promise((r) => setTimeout(r, Math.min(1000, (playhead - ac.currentTime - PCM_MAX_AHEAD_S) * 1000 + 50)));
      }
      if (state.aborted) break;

      const startAt = Math.max(ac.currentTime + LEAD_SECONDS, playhead);
      source.start(startAt);
      playhead = startAt + buffer.duration;
      lastSource = source;
      state.sources.push(source);
      source.onended = () => {
        const i = state.sources.indexOf(source);
        if (i >= 0) state.sources.splice(i, 1);
      };
    }
  } finally {
    reader.cancel().catch(() => {});
  }

  if (state.aborted || !lastSource) {
    if (!state.aborted) gain.disconnect();
    return;
  }

  await new Promise<void>((resolve) => {
    let settled = false;
    const finish = () => {
      if (settled) return;
      settled = true;
      // Release the per-segment graph; otherwise every utterance leaves a
      // GainNode wired to the destination for the rest of the call.
      try {
        gain.disconnect();
      } catch {
        /* already disconnected by stopSpeaking */
      }
      resolve();
    };
    lastSource!.onended = finish;
    // Safety net: onended can be missed if the context is interrupted.
    const remainingMs = Math.max(0, (playhead - ac.currentTime) * 1000) + 300;
    setTimeout(finish, remainingMs);
  });
}

/**
 * MP3/whatever-the-server-sent path, for tiers where PCM isn't available.
 * Decoded and played through the shared AudioContext (unlocked by the user's
 * tap); a fresh <audio> element would need its own gesture on iOS.
 */
async function playEncoded(res: Response, state: PlaybackState): Promise<void> {
  const bytes = await res.arrayBuffer();
  if (state.aborted) return;

  if (await audioContextRunning(AUDIO_RESUME_WAIT_MS)) {
    const ac = audioContext();
    const buffer = await decodeAudio(ac, bytes.slice(0)).catch(() => null);
    if (state.aborted) return;
    if (buffer) {
      await playBuffer(ac, buffer, state);
      return;
    }
  }

  const url = URL.createObjectURL(new Blob([bytes], { type: res.headers.get("Content-Type") || "audio/mpeg" }));
  const audio = new Audio(url);
  state.element = audio;
  if (state.volume !== undefined) audio.volume = state.volume;

  try {
    await audio.play();
  } catch (err) {
    URL.revokeObjectURL(url);
    throw err;
  }

  await new Promise<void>((resolve) => {
    let settled = false;
    const finish = () => {
      if (settled) return;
      settled = true;
      URL.revokeObjectURL(url);
      resolve();
    };
    audio.onended = finish;
    audio.onerror = finish;
  });
}

function decodeAudio(ac: AudioContext, data: ArrayBuffer): Promise<AudioBuffer> {
  // Older Safari only has the callback form.
  return new Promise((resolve, reject) => {
    try {
      const p = ac.decodeAudioData(data, resolve, reject) as Promise<AudioBuffer> | undefined;
      if (p && typeof p.then === "function") p.then(resolve, reject);
    } catch (err) {
      reject(err);
    }
  });
}

async function playBuffer(ac: AudioContext, buffer: AudioBuffer, state: PlaybackState): Promise<void> {
  const gain = ac.createGain();
  if (state.volume !== undefined && gain.gain) gain.gain.value = state.volume;
  gain.connect(ac.destination);
  state.gain = gain;
  const source = ac.createBufferSource();
  source.buffer = buffer;
  source.connect(gain);
  state.sources.push(source);
  await new Promise<void>((resolve) => {
    let settled = false;
    const finish = () => {
      if (settled) return;
      settled = true;
      try {
        gain.disconnect();
      } catch {
        /* already disconnected by cancelAll */
      }
      resolve();
    };
    source.onended = finish;
    source.start();
    setTimeout(finish, buffer.duration * 1000 + 500);
  });
}

// --- Browser (Web Speech) voice ----------------------------------------------------

/** No `onstart` within this long, and the engine isn't speaking → it was blocked. */
export const SPEECH_START_TIMEOUT_MS = 2500;
/** Long text is spoken in pieces: Chrome cuts single utterances off after ~15 s. */
export const SPEECH_CHUNK_MAX = 160;
/** Chrome drops a speak() issued straight after cancel(). */
const AFTER_CANCEL_MS = 120;
/** How long to wait, once, for an asynchronously loaded voice list. */
const VOICE_LIST_WAIT_MS = 1200;

let lastCancelAt = 0;
let voiceListWaited = false;
// Chrome garbage-collects utterances nothing references, and then never fires onend.
const liveUtterances = new Set<SpeechSynthesisUtterance>();

// Legacy tags some platforms (notably Android) still report.
const TAG_ALIASES: Record<string, string> = { iw: "he", in: "id", ji: "yi", no: "nb" };

function normTag(tag: string): string {
  const t = (tag || "").replace(/_/g, "-").toLowerCase();
  const [base, ...rest] = t.split("-");
  return [TAG_ALIASES[base] ?? base, ...rest].join("-");
}

/**
 * The voice that best speaks `lang`: the exact locale beats the same language
 * in another region; the platform default and higher-quality voices win ties.
 * Null when no installed voice speaks the language at all.
 */
export function pickVoice<V extends Pick<SpeechSynthesisVoice, "lang" | "name" | "localService" | "default">>(
  voices: readonly V[],
  lang: string,
  localOnly = false
): V | null {
  const want = normTag(lang);
  const base = want.split("-")[0];
  let best: V | null = null;
  let bestScore = 0;
  for (const v of voices) {
    if (localOnly && !v.localService) continue;
    const tag = normTag(v.lang);
    if (tag.split("-")[0] !== base) continue;
    let score = tag === want ? 100 : 50;
    if (v.default) score += 4;
    if (/natural|neural|premium|enhanced|google/i.test(v.name || "")) score += 2;
    if (score > bestScore) {
      best = v;
      bestScore = score;
    }
  }
  return best;
}

/** Split text into pieces of at most `max` characters, at sentence, then clause, then word boundaries. */
export function splitForSpeech(text: string, max = SPEECH_CHUNK_MAX): string[] {
  const clean = text.replace(/\s+/g, " ").trim();
  if (clean.length <= max) return clean ? [clean] : [];
  const sentences = clean.match(/[^.!?。！？؟।]+[.!?。！？؟।]*\s*/g) ?? [clean];
  const out: string[] = [];
  let cur = "";
  const push = (piece: string) => {
    if ((cur + piece).length <= max) {
      cur += piece;
      return;
    }
    if (cur.trim()) out.push(cur.trim());
    cur = "";
    if (piece.length <= max) {
      cur = piece;
      return;
    }
    // One very long sentence: break at commas or spaces; hard-split if neither (e.g. CJK).
    let rest = piece;
    while (rest.length > max) {
      const window = rest.slice(0, max);
      const at = Math.max(window.lastIndexOf(", "), window.lastIndexOf("、"), window.lastIndexOf("，"), window.lastIndexOf(" "));
      const cut = at > max / 3 ? at + 1 : max;
      out.push(rest.slice(0, cut).trim());
      rest = rest.slice(cut);
    }
    cur = rest;
  };
  for (const sentence of sentences) push(sentence);
  if (cur.trim()) out.push(cur.trim());
  return out.filter(Boolean);
}

function waitForVoices(synth: SpeechSynthesis): Promise<SpeechSynthesisVoice[]> {
  const now = synth.getVoices?.() ?? [];
  if (now.length || voiceListWaited || typeof synth.addEventListener !== "function") return Promise.resolve(now);
  voiceListWaited = true;
  return new Promise((resolve) => {
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      synth.removeEventListener?.("voiceschanged", finish);
      resolve(synth.getVoices?.() ?? []);
    };
    synth.addEventListener("voiceschanged", finish);
    setTimeout(finish, VOICE_LIST_WAIT_MS);
  });
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function finiteOr(v: unknown, fallback: number, lo: number, hi: number): number {
  return typeof v === "number" && Number.isFinite(v) ? Math.min(hi, Math.max(lo, v)) : fallback;
}

/**
 * Speaks with the browser's own voice, in the voice that matches `lang`.
 * Resolves "ok" only if speech actually started; "blocked" when the browser
 * never started it (autoplay rules, no prior tap); "no-voice" when an
 * on-device voice was required and none speaks the language.
 */
async function browserSpeak(
  text: string,
  lang: string,
  state: PlaybackState,
  hints?: RenderHints,
  localOnly = false
): Promise<"ok" | PlaybackFailure> {
  if (typeof window === "undefined" || !("speechSynthesis" in window) || state.aborted) return "error";
  const synth = window.speechSynthesis;
  const voices = await waitForVoices(synth);
  if (state.aborted) return "error";
  const voice = pickVoice(voices, lang, localOnly);
  // On-device only: an explicit localService voice is required (the default voice may be a network voice).
  if (localOnly && !voice) return "no-voice";

  const since = Date.now() - lastCancelAt;
  if (since < AFTER_CANCEL_MS) await sleep(AFTER_CANCEL_MS - since);

  for (const piece of splitForSpeech(text)) {
    if (state.aborted) return "error";
    const r = await speakPiece(synth, piece, lang, voice, state, hints, localOnly);
    if (r !== "ok") return r;
  }
  return "ok";
}

function speakPiece(
  synth: SpeechSynthesis,
  text: string,
  lang: string,
  voice: SpeechSynthesisVoice | null,
  state: PlaybackState,
  hints: RenderHints | undefined,
  localOnly: boolean
): Promise<"ok" | PlaybackFailure> {
  return new Promise((resolve) => {
    let started = false;
    let settled = false;
    const timers: ReturnType<typeof setTimeout>[] = [];
    const utterance = new SpeechSynthesisUtterance(text);
    const finish = (r: "ok" | PlaybackFailure) => {
      if (settled) return;
      settled = true;
      timers.forEach(clearTimeout);
      liveUtterances.delete(utterance);
      resolve(r);
    };
    liveUtterances.add(utterance);

    utterance.lang = lang;
    if (state.volume !== undefined) utterance.volume = state.volume;
    if (hints) {
      // Out-of-range or NaN values throw on assignment in some engines.
      utterance.pitch = finiteOr(hints.browserPitch, 1, 0, 2);
      utterance.rate = finiteOr(hints.browserRate, 1, 0.1, 10);
    }
    try {
      if (voice) utterance.voice = voice;
    } catch {
      // Not a native SpeechSynthesisVoice (e.g. a polyfill): lang selection only.
      if (localOnly) return finish("no-voice");
    }

    utterance.onstart = () => {
      started = true;
    };
    utterance.onend = () => finish("ok");
    utterance.onerror = (e?: Event) => {
      const code = (e as (Event & { error?: string }) | undefined)?.error;
      if (code === "not-allowed") return finish("blocked");
      if (code === "language-unavailable" || code === "voice-unavailable") return finish("no-voice");
      // Cut off after it started (e.g. a later cancel): it was heard.
      if (started && (code === "interrupted" || code === "canceled")) return finish("ok");
      finish("error");
    };

    try {
      if (synth.paused) synth.resume(); // a stuck paused engine queues forever
      synth.speak(utterance);
    } catch {
      return finish("error");
    }

    // Never started and the engine is idle: the browser refused (usually no
    // prior tap on this page). Don't wait out the clip in silence.
    timers.push(
      setTimeout(() => {
        if (started || synth.speaking || synth.pending) return;
        try {
          synth.cancel();
        } catch {
          /* ignore */
        }
        finish("blocked");
      }, SPEECH_START_TIMEOUT_MS)
    );

    // onend is unreliable across browsers: bound the wait, extending while the
    // engine still reports speaking.
    let extensions = 0;
    const budgetMs = Math.max(4000, text.length * 120);
    const check = () => {
      if (state.aborted) {
        try {
          synth.cancel();
        } catch {
          /* ignore */
        }
        return finish("error");
      }
      if (synth.speaking && extensions++ < 3) {
        timers.push(setTimeout(check, 2000));
        return;
      }
      finish(started || synth.speaking ? "ok" : "blocked");
    };
    timers.push(setTimeout(check, budgetMs));
  });
}

/** Chrome populates the voice list asynchronously; ask for it early. */
export function preloadVoices(): void {
  if (typeof window === "undefined" || !("speechSynthesis" in window)) return;
  const synth = window.speechSynthesis;
  try {
    synth.getVoices();
    synth.addEventListener?.("voiceschanged", () => synth.getVoices());
  } catch {
    /* ignore */
  }
}

/** Test hook: forget server availability and voice-list state. */
export function resetTtsStateForTests(): void {
  serverTtsSkipUntil = 0;
  voiceListWaited = false;
  lastCancelAt = 0;
}
