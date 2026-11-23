// Shared audio plumbing for the call: one AudioContext, one microphone stream,
// and the user-gesture unlock that mobile browsers demand before they will play
// anything. Acquiring the mic once per call (rather than once per push) is what
// keeps the first word of an utterance from being swallowed.

import type { CapturePort } from "../engine/ports";
import type { PermissionState } from "../engine/events";

let ctx: AudioContext | null = null;
let micStream: MediaStream | null = null;
let micRequest: Promise<MediaStream> | null = null;
// Bumped by releaseMic(). A getUserMedia that resolves after the call was left
// (e.g. the permission prompt was answered late) must not resurrect the mic.
let micEpoch = 0;
const deviceLostListeners = new Set<() => void>();

export function audioContext(): AudioContext {
  if (!ctx) {
    const Ctor =
      window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
    ctx = new Ctor();
  }
  return ctx;
}

let speechPrimed = false;

/**
 * Must be called synchronously from inside a real user gesture (any tap).
 * Everything that needs the gesture happens before the first await: iOS only
 * honours the unlock while the tap is still being handled.
 *  - a one-sample buffer flips the AudioContext's autoplay flag;
 *  - one silent utterance unlocks speechSynthesis (separate unlock on iOS).
 * Cheap to call on every tap: after the first, only a suspended context is resumed.
 */
export async function unlockAudio(): Promise<void> {
  let ac: AudioContext | null = null;
  try {
    ac = audioContext();
  } catch {
    ac = null; // no Web Audio: the browser voice may still work
  }
  if (!speechPrimed && typeof window !== "undefined" && "speechSynthesis" in window && typeof SpeechSynthesisUtterance !== "undefined") {
    try {
      // A single space: an empty string is skipped without unlocking on some iOS versions.
      const u = new SpeechSynthesisUtterance(" ");
      u.volume = 0;
      window.speechSynthesis.speak(u);
      speechPrimed = true;
    } catch {
      /* ignore */
    }
  }
  if (!ac) return;
  const resumed = ac.state !== "running" ? ac.resume().catch(() => {}) : null;
  try {
    const src = ac.createBufferSource();
    src.buffer = ac.createBuffer(1, 1, 22050);
    src.connect(ac.destination);
    src.start(0);
  } catch {
    // Non-fatal: worst case the first utterance needs a tap.
  }
  if (resumed) await resumed;
}

/** Unlock audio on every tap or key press anywhere in the app (capture phase). */
export function installAudioUnlock(target: Pick<Document, "addEventListener"> = document): void {
  const unlock = () => void unlockAudio();
  for (const type of ["pointerdown", "touchend", "keydown"] as const) {
    target.addEventListener(type, unlock, { capture: true, passive: true });
  }
}

/**
 * The call's microphone stream. Held open for the duration of the call and
 * gated with `track.enabled` rather than reacquired per push — getUserMedia
 * costs 200-500ms, which is exactly the window the first word lives in.
 */
export async function getMic(): Promise<MediaStream> {
  if (micStream && micStream.getAudioTracks().some((t) => t.readyState === "live")) {
    return micStream;
  }
  if (!micRequest) {
    const epoch = micEpoch;
    micRequest = navigator.mediaDevices
      .getUserMedia({
        audio: {
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
      })
      .then((s) => {
        if (epoch !== micEpoch) {
          s.getTracks().forEach((t) => t.stop());
          throw new DOMException("Microphone released before it was acquired", "AbortError");
        }
        micStream = s;
        // 'ended' fires when the device goes away (unplugged, revoked, taken
        // by another app) — never for our own stop() calls.
        s.getAudioTracks().forEach((t) =>
          t.addEventListener?.("ended", () => {
            if (micStream !== s) return;
            micStream = null;
            deviceLostListeners.forEach((fn) => fn());
          })
        );
        return s;
      })
      .finally(() => {
        if (micRequest === request) micRequest = null;
      });
    const request = micRequest;
  }
  return micRequest;
}

/** The call's microphone stream if it is open and enabled right now (server-STT path only). */
export function liveMicStream(): MediaStream | null {
  return micStream && micStream.getAudioTracks().some((t) => t.readyState === "live" && t.enabled) ? micStream : null;
}

export function setMicEnabled(enabled: boolean) {
  micStream?.getAudioTracks().forEach((t) => {
    t.enabled = enabled;
  });
}

export function releaseMic() {
  micEpoch++;
  micStream?.getTracks().forEach((t) => t.stop());
  micStream = null;
  micRequest = null;
}

/**
 * Polls input loudness (0..1) so the UI can show the user their voice is
 * actually being picked up — the single most useful bit of feedback when a
 * push-to-talk app appears to do nothing.
 */
export function createLevelMeter(stream: MediaStream, onLevel: (level: number) => void) {
  const ac = audioContext();
  const source = ac.createMediaStreamSource(stream);
  const analyser = ac.createAnalyser();
  analyser.fftSize = 512;
  analyser.smoothingTimeConstant = 0.6;
  source.connect(analyser);

  const data = new Uint8Array(analyser.frequencyBinCount);
  let raf = 0;
  let stopped = false;

  const tick = () => {
    if (stopped) return;
    analyser.getByteTimeDomainData(data);
    let sum = 0;
    for (let i = 0; i < data.length; i++) {
      const v = (data[i] - 128) / 128;
      sum += v * v;
    }
    const rms = Math.sqrt(sum / data.length);
    onLevel(Math.min(1, rms * 4));
    raf = requestAnimationFrame(tick);
  };
  tick();

  return () => {
    stopped = true;
    cancelAnimationFrame(raf);
    source.disconnect();
    analyser.disconnect();
  };
}

/** Microphone permission without prompting, where the browser supports asking. */
export async function micPermission(): Promise<PermissionState> {
  try {
    const status = await navigator.permissions?.query({ name: "microphone" as PermissionName });
    if (!status) return "unknown";
    return status.state === "granted" ? "granted" : status.state === "denied" ? "denied" : "prompt";
  } catch {
    // Firefox and older Safari reject the "microphone" permission name.
    return "unknown";
  }
}

/** The capture port the engine uses. */
export const captureAdapter: CapturePort = {
  permission: micPermission,
  release: releaseMic,
  onDeviceLost(fn: () => void) {
    deviceLostListeners.add(fn);
    return () => deviceLostListeners.delete(fn);
  },
};
