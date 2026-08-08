// Shared audio plumbing for the call: one AudioContext, one microphone stream,
// and the user-gesture unlock that mobile browsers demand before they will play
// anything. Acquiring the mic once per call (rather than once per push) is what
// keeps the first word of an utterance from being swallowed.

let ctx: AudioContext | null = null;
let micStream: MediaStream | null = null;
let micRequest: Promise<MediaStream> | null = null;

export function audioContext(): AudioContext {
  if (!ctx) {
    const Ctor =
      window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
    ctx = new Ctor();
  }
  return ctx;
}

/**
 * Must be called synchronously from inside a real user gesture (the tap that
 * starts or joins a call). Playing a one-sample buffer is what flips the
 * autoplay flag, so later TTS can start without a fresh tap.
 */
export async function unlockAudio(): Promise<void> {
  const ac = audioContext();
  try {
    const src = ac.createBufferSource();
    src.buffer = ac.createBuffer(1, 1, 22050);
    src.connect(ac.destination);
    src.start(0);
  } catch {
    // Non-fatal: worst case the first utterance needs a tap.
  }
  if (ac.state === "suspended") {
    await ac.resume().catch(() => {});
  }
  // speechSynthesis has its own separate unlock on iOS.
  if ("speechSynthesis" in window) {
    try {
      const u = new SpeechSynthesisUtterance("");
      u.volume = 0;
      window.speechSynthesis.speak(u);
    } catch {
      /* ignore */
    }
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
    micRequest = navigator.mediaDevices
      .getUserMedia({
        audio: {
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
      })
      .then((s) => {
        micStream = s;
        return s;
      })
      .finally(() => {
        micRequest = null;
      });
  }
  return micRequest;
}

export function setMicEnabled(enabled: boolean) {
  micStream?.getAudioTracks().forEach((t) => {
    t.enabled = enabled;
  });
}

export function releaseMic() {
  micStream?.getTracks().forEach((t) => t.stop());
  micStream = null;
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
