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

interface PlaybackState {
  aborted: boolean;
  sources: AudioBufferSourceNode[];
  gain: GainNode | null;
  element: HTMLAudioElement | null;
}

let current: PlaybackState | null = null;
let queueTail: Promise<void> = Promise.resolve();
let queueDepth = 0;

export function isSpeaking(): boolean {
  return queueDepth > 0;
}

/**
 * Queues one segment. Segments play back to back in call order; the returned
 * promise settles when this segment finishes (or is cut off).
 */
export function enqueueSpeech(
  text: string,
  lang: string,
  hooks?: { onStart?: () => void; onEnd?: () => void }
): Promise<void> {
  const clean = text.trim();
  if (!clean) return Promise.resolve();

  queueDepth++;
  const run = queueTail.then(async () => {
    try {
      hooks?.onStart?.();
      await playOne(clean, lang);
    } catch (err) {
      console.error("TTS playback failed:", err);
    } finally {
      queueDepth = Math.max(0, queueDepth - 1);
      hooks?.onEnd?.();
    }
  });
  // Keep the chain alive even if one segment throws.
  queueTail = run.catch(() => {});
  return run;
}

/** Barge-in: kill whatever is playing and drop the rest of the queue. */
export function stopSpeaking() {
  if (current) {
    current.aborted = true;
    current.sources.forEach((s) => {
      try {
        s.stop();
      } catch {
        /* already finished */
      }
    });
    try {
      current.gain?.disconnect();
    } catch {
      /* ignore */
    }
    if (current.element) {
      current.element.pause();
      current.element.src = "";
    }
    current = null;
  }
  if ("speechSynthesis" in window) {
    try {
      window.speechSynthesis.cancel();
    } catch {
      /* ignore */
    }
  }
  queueDepth = 0;
  queueTail = Promise.resolve();
}

async function playOne(text: string, lang: string): Promise<void> {
  const state: PlaybackState = { aborted: false, sources: [], gain: null, element: null };
  current = state;

  try {
    const res = await fetch(`${API_BASE}/api/tts`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ text, lang }),
    });

    if (!res.ok || !res.body) throw new Error(`TTS request failed: ${res.status}`);

    const format = res.headers.get("X-Audio-Format") || "";
    if (format === "pcm") {
      const rate = Number(res.headers.get("X-Sample-Rate")) || 24000;
      await playPcmStream(res, rate, state);
    } else {
      await playEncoded(res, state);
    }
  } catch (err) {
    if (state.aborted) return;
    console.warn("Falling back to browser speech synthesis:", err);
    await browserSpeak(text, lang, state);
  } finally {
    if (current === state) current = null;
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

      const startAt = Math.max(ac.currentTime + LEAD_SECONDS, playhead);
      source.start(startAt);
      playhead = startAt + buffer.duration;
      lastSource = source;
      state.sources.push(source);
    }
  } finally {
    reader.cancel().catch(() => {});
  }

  if (state.aborted || !lastSource) return;

  await new Promise<void>((resolve) => {
    let settled = false;
    const finish = () => {
      if (settled) return;
      settled = true;
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
function browserSpeak(text: string, lang: string, state: PlaybackState): Promise<void> {
  if (!("speechSynthesis" in window)) return Promise.resolve();

  return new Promise<void>((resolve) => {
    let settled = false;
    const finish = () => {
      if (settled) return;
      settled = true;
      resolve();
    };

    const utterance = new SpeechSynthesisUtterance(text);
    utterance.lang = lang;

    const voice = window.speechSynthesis
      .getVoices()
      .find((v) => v.lang === lang) ??
      window.speechSynthesis.getVoices().find((v) => v.lang.startsWith(lang.split("-")[0]));
    if (voice) utterance.voice = voice;

    utterance.onend = finish;
    utterance.onerror = finish;
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
