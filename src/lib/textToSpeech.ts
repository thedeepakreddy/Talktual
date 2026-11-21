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

import type { PlaybackItem, PlaybackListener, PlaybackPort } from "../engine/ports";
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

  private emit(id: string, status: PlaybackStatus, engine?: Engine) {
    for (const l of this.listeners) {
      try {
        l(id, status, engine);
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
        this.playing = item;
        const state: PlaybackState = {
          aborted: false,
          controller: new AbortController(),
          sources: [],
          gain: null,
          element: null,
        };
        this.current = state;
        item.hooks?.onStart?.();
        this.emit(item.segmentId, "started");
        let outcome: Engine | "none" | "cancelled" = "none";
        try {
          outcome = await playOne(item.text, item.lang, state, safeHints(item.voice));
        } catch (err) {
          console.error("TTS playback failed:", err);
          outcome = state.aborted ? "cancelled" : "none";
        }
        if (this.current === state) this.current = null;
        const wasCancelled = state.aborted || this.playing !== item;
        if (this.playing === item) this.playing = null;
        if (!wasCancelled) {
          if (outcome === "none") this.emit(item.segmentId, "failed");
          else this.emit(item.segmentId, "ended", outcome === "cancelled" ? undefined : outcome);
        }
        item.hooks?.onEnd?.();
        item.done();
      }
    } finally {
      this.pumping = false;
    }
  }
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

async function playOne(
  text: string,
  lang: string,
  state: PlaybackState,
  hints?: RenderHints
): Promise<Engine | "none" | "cancelled"> {
  try {
    const res = await fetch(`${API_BASE}/api/tts`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      // Only coarse, non-identifying hints go to the server; it picks the voice.
      body: JSON.stringify(hints ? { text, lang, voice: { register: hints.register, speed: hints.ttsSpeed } } : { text, lang }),
      signal: state.controller.signal,
    });

    if (!res.ok || !res.body) throw new Error(`TTS request failed: ${res.status}`);

    const format = res.headers.get("X-Audio-Format") || "";
    if (format === "pcm") {
      const rate = Number(res.headers.get("X-Sample-Rate")) || 24000;
      await playPcmStream(res, rate, state);
    } else {
      await playEncoded(res, state);
    }
    return state.aborted ? "cancelled" : "server";
  } catch (err) {
    if (state.aborted) return "cancelled";
    console.warn("Falling back to browser speech synthesis:", err);
    const spoke = await browserSpeak(text, lang, state, hints);
    if (state.aborted) return "cancelled";
    return spoke ? "browser" : "none";
  }
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

      // Backpressure: don't schedule more than PCM_MAX_AHEAD_S into the future.
      // The reader (and so the network stream) waits instead of buffering
      // the whole clip in audio nodes.
      while (!state.aborted && playhead - ac.currentTime > PCM_MAX_AHEAD_S) {
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

/** MP3/whatever-the-server-sent path, for tiers where PCM isn't available. */
async function playEncoded(res: Response, state: PlaybackState): Promise<void> {
  const blob = await res.blob();
  if (state.aborted) return;

  const url = URL.createObjectURL(blob);
  const audio = new Audio(url);
  state.element = audio;

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

/** Last resort so a call never goes silent, even with no API key configured. */
function browserSpeak(text: string, lang: string, state: PlaybackState, hints?: RenderHints): Promise<boolean> {
  if (!("speechSynthesis" in window) || state.aborted) return Promise.resolve(false);

  return new Promise<boolean>((resolve) => {
    let settled = false;
    const finish = (ok = true) => {
      if (settled) return;
      settled = true;
      resolve(ok);
    };

    const utterance = new SpeechSynthesisUtterance(text);
    utterance.lang = lang;
    if (hints) {
      utterance.pitch = hints.browserPitch;
      utterance.rate = hints.browserRate;
    }

    const voice = window.speechSynthesis
      .getVoices()
      .find((v) => v.lang === lang) ??
      window.speechSynthesis.getVoices().find((v) => v.lang.startsWith(lang.split("-")[0]));
    if (voice) utterance.voice = voice;

    utterance.onend = () => finish(true);
    utterance.onerror = () => finish(false);
    window.speechSynthesis.speak(utterance);

    // speechSynthesis fires onend unreliably across browsers; bound the wait.
    const budgetMs = Math.max(4000, text.length * 120);
    setTimeout(() => {
      if (state.aborted) window.speechSynthesis.cancel();
      finish();
    }, budgetMs);
  });
}

/** Chrome populates the voice list asynchronously; ask for it early. */
export function preloadVoices(): void {
  if (!("speechSynthesis" in window)) return;
  window.speechSynthesis.getVoices();
  window.speechSynthesis.onvoiceschanged = () => {
    window.speechSynthesis.getVoices();
  };
}
