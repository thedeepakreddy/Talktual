// Push-to-talk speech recognition.
//
// The whole point of this module is that transcription finishes *while* the
// user is still holding the button, not after they let go. The Web Speech API
// gives us that for free: it streams interim text continuously and finalizes
// each phrase at natural pauses, so a long utterance turns into several
// segments that we can translate and ship one at a time.
//
// Browsers without Web Speech (Firefox, mostly) fall back to recording the
// whole push and posting it to ElevenLabs Scribe on release. That path is
// correct but not instant — roughly a second of upload-and-transcribe.

import { API_BASE, RELEASE_TAIL_MS } from "./config";
import { getMic } from "./audio";

export type SttEngine = "webspeech" | "scribe";

export interface RecognizerCallbacks {
  /** A finalized phrase, ready to translate and send. */
  onSegment: (text: string) => void;
  /** Live interim text for on-screen captions. Fires often; cheap to ignore. */
  onPartial: (text: string) => void;
  /** The mic is picking up actual speech (Web Speech engine only). */
  onSpeechActivity?: (active: boolean) => void;
  onError: (message: string, fatal: boolean) => void;
}

export interface Recognizer {
  engine: SttEngine;
  /** Begin listening for the call. Safe to call more than once. */
  arm: () => Promise<void>;
  /** User pressed the PTT button. */
  hold: () => void;
  /** User released the PTT button. */
  release: () => void;
  destroy: () => void;
}

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

export function createRecognizer(lang: string, cb: RecognizerCallbacks): Recognizer {
  return getSpeechRecognitionCtor()
    ? createWebSpeechRecognizer(lang, cb)
    : createScribeRecognizer(lang, cb);
}

// --- Web Speech engine (the fast path) --------------------------------------

function createWebSpeechRecognizer(lang: string, cb: RecognizerCallbacks): Recognizer {
  const Ctor = getSpeechRecognitionCtor()!;
  let rec: SpeechRecognitionLike | null = null;
  let armed = false;
  let holding = false;
  let tailUntil = 0;
  let pendingInterim = "";
  let lastEmitted = "";
  let lastEmittedAt = 0;
  let restartTimer: ReturnType<typeof setTimeout> | null = null;
  let releaseTimer: ReturnType<typeof setTimeout> | null = null;

  /** Results only count while the button is down, plus a short trailing grace. */
  const accepting = () => holding || Date.now() < tailUntil;

  function emit(text: string) {
    const clean = text.trim();
    if (!clean) return;
    // Chrome occasionally replays the last final result when a session ends.
    if (clean === lastEmitted && Date.now() - lastEmittedAt < 1500) return;
    lastEmitted = clean;
    lastEmittedAt = Date.now();
    cb.onSegment(clean);
  }

  function build(): SpeechRecognitionLike {
    const r = new Ctor();
    r.lang = lang;
    r.continuous = true;
    r.interimResults = true;
    r.maxAlternatives = 1;

    r.onresult = (event) => {
      let interim = "";
      for (let i = event.resultIndex; i < event.results.length; i++) {
        const result = event.results[i];
        const text = result[0]?.transcript ?? "";
        if (result.isFinal) {
          if (accepting()) emit(text);
        } else {
          interim += text;
        }
      }
      pendingInterim = interim;
      if (accepting()) cb.onPartial(interim.trim());
    };

    r.onspeechstart = () => {
      if (accepting()) cb.onSpeechActivity?.(true);
    };
    r.onspeechend = () => cb.onSpeechActivity?.(false);

    r.onerror = (event) => {
      const code = (event as Event & { error?: string }).error || "unknown";
      // These are routine in continuous mode and just mean "restart me".
      if (code === "no-speech" || code === "aborted" || code === "audio-capture") return;
      if (code === "not-allowed" || code === "service-not-allowed") {
        armed = false;
        cb.onError("Microphone permission denied for speech recognition.", true);
        return;
      }
      cb.onError(`Speech recognition error: ${code}`, false);
    };

    r.onend = () => {
      pendingInterim = "";
      cb.onSpeechActivity?.(false);
      // Chrome ends sessions on its own every so often; keep it alive.
      if (armed) scheduleRestart(150);
    };

    return r;
  }

  function scheduleRestart(delay: number) {
    if (restartTimer) clearTimeout(restartTimer);
    restartTimer = setTimeout(() => {
      restartTimer = null;
      if (!armed) return;
      try {
        rec?.start();
      } catch {
        // Already started, or start() raced with end(); try again shortly.
        scheduleRestart(400);
      }
    }, delay);
  }

  return {
    engine: "webspeech",

    async arm() {
      if (armed) return;
      armed = true;
      rec = build();
      try {
        rec.start();
      } catch {
        scheduleRestart(300);
      }
    },

    hold() {
      holding = true;
      tailUntil = 0;
      pendingInterim = "";
      if (armed && !rec) {
        rec = build();
        try {
          rec.start();
        } catch {
          scheduleRestart(200);
        }
      }
    },

    release() {
      holding = false;
      tailUntil = Date.now() + RELEASE_TAIL_MS;
      const hadInterim = pendingInterim.trim().length > 0;

      if (releaseTimer) clearTimeout(releaseTimer);
      releaseTimer = setTimeout(() => {
        releaseTimer = null;
        cb.onSpeechActivity?.(false);
        cb.onPartial("");
        // If a phrase never finalized on its own, stop() forces the engine to
        // flush it. onend then restarts us, and that restart happens while the
        // translation is already in flight — so the user never waits on it.
        if (hadInterim && pendingInterim.trim()) {
          try {
            rec?.stop();
          } catch {
            /* already stopped */
          }
        }
      }, RELEASE_TAIL_MS);
    },

    destroy() {
      armed = false;
      holding = false;
      if (restartTimer) clearTimeout(restartTimer);
      if (releaseTimer) clearTimeout(releaseTimer);
      try {
        rec?.abort();
      } catch {
        /* ignore */
      }
      rec = null;
    },
  };
}

// --- ElevenLabs Scribe engine (the compatibility path) ----------------------

function pickMimeType(): string {
  const candidates = ["audio/webm;codecs=opus", "audio/webm", "audio/mp4", "audio/ogg;codecs=opus"];
  for (const type of candidates) {
    if (typeof MediaRecorder !== "undefined" && MediaRecorder.isTypeSupported(type)) return type;
  }
  return "";
}

function createScribeRecognizer(lang: string, cb: RecognizerCallbacks): Recognizer {
  let stream: MediaStream | null = null;
  let active: MediaRecorder | null = null;
  let armed = false;
  // True between hold() and release(). hold() is async (it may still be
  // waiting on the mic), so a quick tap can release before recording starts;
  // without this flag that recorder would then run until the next release.
  let holding = false;

  async function ensureStream(): Promise<MediaStream | null> {
    if (stream && stream.getAudioTracks().some((t) => t.readyState === "live")) return stream;
    try {
      stream = await getMic();
      return stream;
    } catch (err) {
      if (armed) cb.onError(`Microphone unavailable: ${String(err)}`, true);
      return null;
    }
  }

  async function upload(collected: Blob[], type: string) {
    const blob = new Blob(collected, { type });
    // Sub-100ms blobs are almost always an accidental tap.
    if (blob.size < 2000) {
      cb.onPartial("");
      return;
    }

    try {
      const ext = type.includes("mp4") ? "mp4" : type.includes("ogg") ? "ogg" : "webm";
      const form = new FormData();
      form.append("file", blob, `push.${ext}`);
      form.append("language_code", lang.split("-")[0]);

      const res = await fetch(`${API_BASE}/api/stt`, { method: "POST", body: form });
      if (!armed) return;
      if (!res.ok) {
        cb.onError(`Transcription failed (${res.status}).`, false);
        return;
      }
      const data = await res.json();
      if (!armed) return;
      const text = (data.text || "").trim();
      if (text) cb.onSegment(text);
      else cb.onError("Nothing was transcribed — try holding a moment longer.", false);
    } catch (err) {
      if (armed) cb.onError(`Transcription network error: ${String(err)}`, false);
    } finally {
      if (armed) cb.onPartial("");
    }
  }

  /** One MediaRecorder per push, each with its own chunk list. */
  function startRecording(s: MediaStream) {
    const mimeType = pickMimeType();
    const recorder = mimeType ? new MediaRecorder(s, { mimeType }) : new MediaRecorder(s);
    const chunks: Blob[] = [];
    recorder.ondataavailable = (e) => {
      if (e.data.size > 0) chunks.push(e.data);
    };
    recorder.onstop = () => {
      if (active === recorder) active = null;
      // Leaving the call mid-push must not upload (and send) that audio.
      if (!armed || chunks.length === 0) return;
      void upload(chunks, recorder.mimeType || mimeType || "audio/webm");
    };
    recorder.start();
    active = recorder;
    cb.onSpeechActivity?.(true);
  }

  return {
    engine: "scribe",

    async arm() {
      armed = true;
      // Warm the mic now so the push itself is instant.
      await ensureStream();
    },

    hold() {
      if (!armed || holding) return;
      holding = true;
      void (async () => {
        const s = await ensureStream();
        if (!s || !armed || !holding || active) return;
        startRecording(s);
      })();
    },

    release() {
      holding = false;
      cb.onSpeechActivity?.(false);
      if (active && active.state === "recording") active.stop();
    },

    destroy() {
      armed = false;
      holding = false;
      try {
        if (active && active.state !== "inactive") active.stop();
      } catch {
        /* ignore */
      }
      active = null;
    },
  };
}
