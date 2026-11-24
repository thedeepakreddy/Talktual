// Speech recognition adapters (SpeechRecognitionAdapter).
//
// Contract (engine/ports.ts): a recognizer listens ONLY between start() and
// stop()/abort(). Nothing here starts on its own. That is what makes the
// listening mode enforceable:
//   * push-to-talk — the session calls start() on press and stop() on release;
//     stop() keeps accepting for a short tail, forces the engine to flush the
//     last phrase, then ends for good;
//   * continuous — the session calls start() when the user turns it on and
//     stop()/abort() when they turn it off; while active, an engine that ends
//     by itself (Chrome does this every so often) is restarted.
//
// Phase 1 kept the Web Speech recognizer running for the whole call so the
// first word wasn't clipped; that streamed audio to the browser vendor while
// the user wasn't talking (Phase 1 SEC-R1). It no longer does.
//
// Two engines:
//   * webspeech — the browser's streaming recognizer (interim + final text).
//   * server-stt — records the push and posts it to our server (ElevenLabs
//     Scribe) on release. Browsers without Web Speech (Firefox, mostly).

import { API_BASE, ASR_ENGINE, FINALIZE_TIMEOUT_MS, RELEASE_TAIL_MS } from "./config";
import { createCeAdapter } from "./ceSpeech";
import { getMic, setMicEnabled } from "./audio";
import type { AsrAdapter, AsrCallbacks, AsrEngine, AsrErrorCode, AsrOptions } from "../engine/ports";
import type { ListeningMode, ProcessingRoute } from "../engine/events";

export type SttEngine = AsrEngine;

/** A single server-STT recording is cut off (and sent) after this long. */
export const MAX_RECORDING_MS = 60_000;

// --- Minimal Web Speech typings (not in lib.dom for all TS versions) ---------

interface SpeechRecognitionAlternativeLike {
  transcript: string;
  confidence: number;
}
interface SpeechRecognitionResultLike {
  readonly length: number;
  isFinal: boolean;
  [index: number]: SpeechRecognitionAlternativeLike;
}
interface SpeechRecognitionResultListLike {
  readonly length: number;
  [index: number]: SpeechRecognitionResultLike;
}
interface SpeechRecognitionEventLike extends Event {
  resultIndex: number;
  results: SpeechRecognitionResultListLike;
}
interface SpeechRecognitionLike extends EventTarget {
  lang: string;
  continuous: boolean;
  interimResults: boolean;
  maxAlternatives: number;
  start(): void;
  stop(): void;
  abort(): void;
  onresult: ((e: SpeechRecognitionEventLike) => void) | null;
  onerror: ((e: Event & { error?: string }) => void) | null;
  onend: (() => void) | null;
  onspeechstart: (() => void) | null;
  onspeechend: (() => void) | null;
}
type SpeechRecognitionCtor = new () => SpeechRecognitionLike;

function getSpeechRecognitionCtor(): SpeechRecognitionCtor | null {
  const w = window as unknown as {
    SpeechRecognition?: SpeechRecognitionCtor;
    webkitSpeechRecognition?: SpeechRecognitionCtor;
  };
  return w.SpeechRecognition || w.webkitSpeechRecognition || null;
}

export function supportsStreamingStt(): boolean {
  return getSpeechRecognitionCtor() !== null;
}

// --- Public factory ---------------------------------------------------------

export function createAsr(lang: string, mode: ListeningMode, cb: AsrCallbacks, opts: AsrOptions = {}): AsrAdapter {
  // Communication Engine recognizer (opt-in build flag): audio goes to the
  // local CE bridge only.
  if (ASR_ENGINE === "ce") return createCeAdapter(lang, mode, cb);
  // On-device only: never fall back to the server recognizer.
  if (opts.processLocally) return createWebSpeechAdapter(lang, mode, cb, true);
  return getSpeechRecognitionCtor() ? createWebSpeechAdapter(lang, mode, cb) : createServerSttAdapter(lang, cb);
}

/** service-not-allowed is ambiguous in Safari: denied mic, or a language its service can't do. */
async function classifyServiceRefusal(code: string): Promise<"permission-denied" | "language-unsupported"> {
  if (code === "language-not-supported") return "language-unsupported";
  try {
    const st = await navigator.permissions?.query({ name: "microphone" as PermissionName });
    if (st?.state === "denied") return "permission-denied";
  } catch {
    /* permission name unsupported: fall through */
  }
  return "language-unsupported";
}

// --- Web Speech engine (the fast path) --------------------------------------

function createWebSpeechAdapter(lang: string, mode: ListeningMode, cb: AsrCallbacks, processLocally = false): AsrAdapter {
  const Ctor = getSpeechRecognitionCtor()!;
  let rec: SpeechRecognitionLike | null = null;
  let active = false; // between start() and the end of stop()/abort()
  let stopping = false;
  let lastEmitted = "";
  let lastEmittedAt = 0;
  let phraseStartedAt = 0;
  let restartTimer: ReturnType<typeof setTimeout> | null = null;
  let tailTimer: ReturnType<typeof setTimeout> | null = null;
  let finalizeTimer: ReturnType<typeof setTimeout> | null = null;

  const clearTimers = () => {
    [restartTimer, tailTimer, finalizeTimer].forEach((t) => t && clearTimeout(t));
    restartTimer = tailTimer = finalizeTimer = null;
  };

  /**
   * Latest NOT-yet-final text of the current phrase. Cloud recognizers often
   * deliver the final result late after stop() — or never (Chrome on Android
   * frequently ends without one). Reliability fix: when the engine ends or the
   * finalize timer fires on a graceful stop, this text is promoted to a final
   * instead of being discarded. An explicit abort (mic off, leave, mode change)
   * still discards it.
   */
  let pendingInterim = "";

  function flushPendingInterim() {
    const t = pendingInterim.trim();
    pendingInterim = "";
    if (!t) return;
    emit(t, undefined, true);
  }

  function emit(text: string, confidence?: number, promoted = false) {
    const clean = text.trim();
    if (!clean) return;
    // Chrome occasionally replays the last final result when a session ends.
    if (clean === lastEmitted && Date.now() - lastEmittedAt < 1500) return;
    lastEmitted = clean;
    lastEmittedAt = Date.now();
    const endedAt = Date.now();
    cb.onFinal({
      text: clean,
      lang,
      startedAt: phraseStartedAt || endedAt,
      endedAt,
      engine: "webspeech",
      // Many browsers report 0 ("unknown"); the conversation layer treats 0 as absent.
      ...(typeof confidence === "number" && Number.isFinite(confidence) ? { confidence } : {}),
      ...(promoted ? { promoted: true } : {}),
    });
    phraseStartedAt = endedAt;
  }

  function finish() {
    if (!active) return;
    active = false;
    stopping = false;
    clearTimers();
    detach();
    cb.onSpeechActivity?.(false);
    cb.onPartial("");
    cb.onEnd?.();
  }

  function detach() {
    if (!rec) return;
    rec.onresult = rec.onerror = rec.onend = rec.onspeechstart = rec.onspeechend = null;
    rec = null;
  }

  function build(): SpeechRecognitionLike {
    const r = new Ctor();
    r.lang = lang;
    if (processLocally) (r as unknown as { processLocally: boolean }).processLocally = true;
    r.continuous = true;
    r.interimResults = true;
    r.maxAlternatives = 1;

    r.onresult = (event) => {
      if (!active) return;
      let interim = "";
      for (let i = event.resultIndex; i < event.results.length; i++) {
        const result = event.results[i];
        const text = result[0]?.transcript ?? "";
        if (result.isFinal) {
          emit(text, result[0]?.confidence);
          pendingInterim = "";
        }
        else interim += text;
      }
      pendingInterim = interim.trim();
      cb.onPartial(interim.trim());
    };

    r.onspeechstart = () => {
      if (active) cb.onSpeechActivity?.(true);
    };
    r.onspeechend = () => cb.onSpeechActivity?.(false);

    r.onerror = (event) => {
      const code = (event as Event & { error?: string }).error || "unknown";
      // Routine in continuous recognition: the engine ends and we restart it.
      if (code === "no-speech" || code === "aborted") return;
      if (code === "not-allowed") {
        cb.onError("permission-denied", "Microphone permission denied for speech recognition.", true);
        abortNow();
        return;
      }
      // "language-not-supported" (Chrome) and "service-not-allowed" (Safari, when
      // its speech service can't recognise the requested language) are NOT a
      // microphone problem. Only report a permission problem if the browser
      // actually says the microphone is denied.
      if (code === "language-not-supported" || code === "service-not-allowed") {
        abortNow();
        void classifyServiceRefusal(code).then((c) =>
          c === "permission-denied"
            ? cb.onError("permission-denied", "Microphone permission denied for speech recognition.", true)
            : cb.onError("language-unsupported", `Speech recognition is not available for ${lang} here.`, true)
        );
        return;
      }
      if (code === "audio-capture") {
        cb.onError("capture-unavailable", "No microphone could be opened.", true);
        abortNow();
        return;
      }
      const mapped: AsrErrorCode = code === "network" ? "network" : "provider";
      cb.onError(mapped, `Speech recognition error: ${code}`, false);
    };

    r.onend = () => {
      if (!active) return;
      // Words the engine never finalized are still the user's words.
      flushPendingInterim();
      if (stopping) {
        finish();
        return;
      }
      // The engine ended on its own while we're supposed to listen
      // (Chrome does this periodically): keep it alive.
      cb.onSpeechActivity?.(false);
      scheduleRestart(150);
    };

    return r;
  }

  function scheduleRestart(delay: number) {
    if (restartTimer) clearTimeout(restartTimer);
    restartTimer = setTimeout(() => {
      restartTimer = null;
      if (!active || stopping) return;
      try {
        rec = build();
        rec.start();
      } catch {
        scheduleRestart(400);
      }
    }, delay);
  }

  function abortNow() {
    const r = rec;
    const wasActive = active;
    pendingInterim = ""; // abort = discard (privacy: nothing captured after a stop is sent)
    active = false;
    stopping = false;
    clearTimers();
    detach();
    try {
      r?.abort();
    } catch {
      /* already stopped */
    }
    if (wasActive) {
      cb.onSpeechActivity?.(false);
      cb.onPartial("");
    }
  }

  return {
    engine: "webspeech",
    route: "browser-speech" as ProcessingRoute,
    get active() {
      return active;
    },

    async start() {
      if (active && !stopping) return;
      if (stopping) {
        // Pressed again during the release tail: keep the running engine.
        stopping = false;
        clearTimers();
        return;
      }
      active = true;
      stopping = false;
      phraseStartedAt = Date.now();
      try {
        rec = build();
        rec.start();
      } catch {
        scheduleRestart(200);
      }
    },

    stop() {
      if (!active || stopping) return;
      stopping = true;
      if (mode === "continuous") {
        // No tail in continuous mode: flush and end now.
        try {
          rec?.stop();
        } catch {
          finish();
          return;
        }
      } else {
        // Keep accepting trailing words for a moment, then make the engine
        // flush the last phrase.
        tailTimer = setTimeout(() => {
          tailTimer = null;
          try {
            if (rec) rec.stop();
            else finish();
          } catch {
            finish();
          }
        }, RELEASE_TAIL_MS);
      }
      // Never wait forever for an engine that doesn't end.
      finalizeTimer = setTimeout(() => {
        finalizeTimer = null;
        if (!active) return;
        // The engine didn't flush in time: use what it had recognized so far.
        flushPendingInterim();
        const r = rec;
        detach();
        try {
          r?.abort();
        } catch {
          /* ignore */
        }
        finish();
      }, (mode === "continuous" ? 0 : RELEASE_TAIL_MS) + FINALIZE_TIMEOUT_MS);
    },

    abort() {
      abortNow();
    },

    dispose() {
      abortNow();
    },
  };
}

// --- Server STT engine (the compatibility path) ----------------------------

function pickMimeType(): string {
  const candidates = ["audio/webm;codecs=opus", "audio/webm", "audio/mp4", "audio/ogg;codecs=opus"];
  for (const type of candidates) {
    if (typeof MediaRecorder !== "undefined" && MediaRecorder.isTypeSupported(type)) return type;
  }
  return "";
}

function captureErrorCode(err: unknown): AsrErrorCode {
  const name = (err as { name?: string })?.name;
  if (name === "NotAllowedError" || name === "SecurityError") return "permission-denied";
  return "capture-unavailable";
}

function createServerSttAdapter(lang: string, cb: AsrCallbacks): AsrAdapter {
  let active = false;
  let stopRequested = false;
  let recorder: MediaRecorder | null = null;
  let capTimer: ReturnType<typeof setTimeout> | null = null;
  let upload: AbortController | null = null;
  // Bumped by abort(); work started under an older generation is discarded.
  let generation = 0;

  function finish() {
    if (!active) return;
    active = false;
    stopRequested = false;
    if (capTimer) clearTimeout(capTimer);
    capTimer = null;
    setMicEnabled(false);
    cb.onSpeechActivity?.(false);
    cb.onEnd?.();
  }

  async function send(chunks: Blob[], type: string, startedAt: number, gen: number) {
    const blob = new Blob(chunks, { type });
    // Sub-100ms blobs are almost always an accidental tap.
    if (blob.size < 2000) {
      cb.onPartial("");
      finish();
      return;
    }
    upload = new AbortController();
    try {
      const ext = type.includes("mp4") ? "mp4" : type.includes("ogg") ? "ogg" : "webm";
      const form = new FormData();
      form.append("file", blob, `push.${ext}`);
      form.append("language_code", lang.split("-")[0]);

      const res = await fetch(`${API_BASE}/api/stt`, { method: "POST", body: form, signal: upload.signal });
      if (gen !== generation) return;
      if (!res.ok) {
        cb.onError(res.status === 429 ? "provider" : "provider", `Transcription failed (${res.status}).`, false);
        return;
      }
      const data = await res.json();
      if (gen !== generation) return;
      const text = (data.text || "").trim();
      if (text) cb.onFinal({ text, lang, startedAt, endedAt: Date.now(), engine: "server-stt" });
      else cb.onError("no-result", "Nothing was transcribed — try holding a moment longer.", false);
    } catch (err) {
      if (gen === generation && (err as Error)?.name !== "AbortError") {
        cb.onError("network", `Transcription network error: ${String(err)}`, false);
      }
    } finally {
      upload = null;
      if (gen === generation) {
        cb.onPartial("");
        finish();
      }
    }
  }

  return {
    engine: "server-stt",
    route: "server-stt" as ProcessingRoute,
    get active() {
      return active;
    },

    async start() {
      if (active) return;
      active = true;
      stopRequested = false;
      const gen = generation;
      let stream: MediaStream;
      try {
        stream = await getMic();
      } catch (err) {
        if (gen !== generation) return;
        active = false;
        cb.onError(captureErrorCode(err), `Microphone unavailable: ${String(err)}`, true);
        throw err;
      }
      // Released (quick tap) or aborted while the mic was being acquired.
      if (gen !== generation || !active) return;
      if (stopRequested) {
        finish();
        return;
      }
      setMicEnabled(true);
      const mimeType = pickMimeType();
      const r = mimeType ? new MediaRecorder(stream, { mimeType }) : new MediaRecorder(stream);
      const chunks: Blob[] = [];
      const startedAt = Date.now();
      r.ondataavailable = (e) => {
        if (e.data.size > 0) chunks.push(e.data);
      };
      r.onstop = () => {
        if (recorder === r) recorder = null;
        if (gen !== generation) return; // aborted: discard the audio
        void send(chunks, r.mimeType || mimeType || "audio/webm", startedAt, gen);
      };
      recorder = r;
      r.start();
      cb.onSpeechActivity?.(true);
      // Bound the buffer: a stuck button can't record forever.
      capTimer = setTimeout(() => {
        capTimer = null;
        if (recorder === r && r.state === "recording") r.stop();
      }, MAX_RECORDING_MS);
    },

    stop() {
      if (!active) return;
      cb.onSpeechActivity?.(false);
      if (recorder && recorder.state === "recording") {
        recorder.stop(); // onstop → send → finish
      } else {
        // Mic still being acquired: don't start recording when it arrives.
        stopRequested = true;
      }
    },

    abort() {
      generation++;
      upload?.abort();
      const r = recorder;
      recorder = null;
      try {
        if (r && r.state !== "inactive") r.stop();
      } catch {
        /* ignore */
      }
      const wasActive = active;
      active = false;
      stopRequested = false;
      if (capTimer) clearTimeout(capTimer);
      capTimer = null;
      setMicEnabled(false);
      if (wasActive) cb.onSpeechActivity?.(false);
    },

    dispose() {
      this.abort();
    },
  };
}
