// Communication Engine recognizer (engine "ce-local", opt-in).
//
// Streams the call's microphone as 16-bit PCM to the local Talktual ↔ CE
// bridge (communication-engine/adapters/talktual-bridge) over a WebSocket and
// turns CE's transcript revisions into the session's AsrCallbacks. CE does the
// recognition (sherpa-onnx streaming Zipformer, English) on this computer.
//
// Same contract as the other adapters (engine/ports.ts): audio flows only
// between start() and stop()/abort(); stop() keeps a short release tail, then
// asks the bridge to finish everything it was sent and ends on its "end";
// abort() discards. Enabled with VITE_ASR_ENGINE=ce (see config.ts).

import { CE_BRIDGE_URL, RELEASE_TAIL_MS } from "./config";
import { audioContext, getMic, setMicEnabled } from "./audio";
import type { AsrAdapter, AsrCallbacks, AsrErrorCode } from "../engine/ports";
import type { ListeningMode, ProcessingRoute } from "../engine/events";
import { languageUnsupportedMessage } from "../engine/asrMessages";

/** How long a graceful stop may take before we give up waiting for CE. */
export const CE_FINALIZE_TIMEOUT_MS = 8000;
/** Audio captured before the bridge is ready is held, at most this long. */
const PRE_READY_MAX_MS = 3000;

/** A running PCM capture. */
export interface PcmCapture {
  sampleRate: number;
  stop(): void;
}

/** Starts capture; `onPcm` receives mono Float32 chunks at `sampleRate`. */
export type StartPcmCapture = (onPcm: (chunk: Float32Array) => void) => Promise<PcmCapture>;

export interface WebSocketLike {
  binaryType: string;
  readyState: number;
  onopen: (() => void) | null;
  onmessage: ((e: { data: unknown }) => void) | null;
  onerror: (() => void) | null;
  onclose: (() => void) | null;
  send(data: string | ArrayBufferLike): void;
  close(): void;
}

export interface CeSpeechDeps {
  url?: string;
  openSocket?: (url: string) => WebSocketLike;
  startCapture?: StartPcmCapture;
  now?: () => number;
}

/** Messages from the bridge (protocol v1). */
type BridgeMsg =
  | { type: "ready"; adapter: string; model: string | null }
  | { type: "utterance"; utterance: number }
  | { type: "partial"; utterance: number; committed: string; provisional: string }
  | { type: "final"; utterance: number; text: string; reason: string; language: string | null; durationMs: number }
  | { type: "cancelled"; utterance: number; reason: string }
  | { type: "activity"; active: boolean }
  | { type: "warning"; kind: string }
  | { type: "error"; code: string; message: string; fatal: boolean }
  | { type: "end"; aborted: boolean };

/**
 * The model emits upper-case words without punctuation. Show them in
 * sentence case (presentation only; the words are not changed).
 */
export function presentCeText(text: string): string {
  const t = text.trim().toLowerCase().replace(/\s+/g, " ");
  if (!t) return "";
  const fixed = t.replace(/\bi\b/g, "I").replace(/\bi'(m|ll|ve|d)\b/g, "I'$1");
  return fixed.charAt(0).toUpperCase() + fixed.slice(1);
}

function toPcm16(chunk: Float32Array): ArrayBuffer {
  const out = new Int16Array(chunk.length);
  for (let i = 0; i < chunk.length; i++) {
    const v = Math.max(-1, Math.min(1, chunk[i] || 0));
    out[i] = v < 0 ? v * 0x8000 : v * 0x7fff;
  }
  return out.buffer;
}

const WORKLET = `
class CePcmTap extends AudioWorkletProcessor {
  process(inputs) {
    const ch = inputs[0] && inputs[0][0];
    if (ch && ch.length) this.port.postMessage(ch.slice(0));
    return true;
  }
}
registerProcessor('ce-pcm-tap', CePcmTap);
`;

/** Default capture: the call's mic → AudioWorklet (ScriptProcessor fallback). */
export const startMicPcm: StartPcmCapture = async (onPcm) => {
  const stream = await getMic();
  const ctx = audioContext();
  if (ctx.state === "suspended") await ctx.resume().catch(() => undefined);
  const source = ctx.createMediaStreamSource(stream);
  const sink = ctx.createGain();
  sink.gain.value = 0; // keep the graph pulled without playing the mic back
  sink.connect(ctx.destination);
  let node: AudioNode;
  if (ctx.audioWorklet && typeof AudioWorkletNode !== "undefined") {
    const url = URL.createObjectURL(new Blob([WORKLET], { type: "application/javascript" }));
    try {
      await ctx.audioWorklet.addModule(url);
    } finally {
      URL.revokeObjectURL(url);
    }
    const w = new AudioWorkletNode(ctx, "ce-pcm-tap", { numberOfInputs: 1, numberOfOutputs: 1, channelCount: 1 });
    w.port.onmessage = (e: MessageEvent<Float32Array>) => onPcm(e.data);
    node = w;
  } else {
    const sp = ctx.createScriptProcessor(2048, 1, 1);
    sp.onaudioprocess = (e) => onPcm(e.inputBuffer.getChannelData(0).slice(0));
    node = sp;
  }
  source.connect(node);
  node.connect(sink);
  return {
    sampleRate: ctx.sampleRate,
    stop() {
      try {
        source.disconnect();
        node.disconnect();
        sink.disconnect();
      } catch {
        /* already disconnected */
      }
      if (node instanceof AudioWorkletNode) node.port.onmessage = null;
    },
  };
};

function captureErrorCode(err: unknown): AsrErrorCode {
  const name = (err as { name?: string })?.name;
  if (name === "NotAllowedError" || name === "SecurityError") return "permission-denied";
  return "capture-unavailable";
}

export function createCeAdapter(lang: string, mode: ListeningMode, cb: AsrCallbacks, deps: CeSpeechDeps = {}): AsrAdapter {
  const url = deps.url ?? CE_BRIDGE_URL;
  const openSocket = deps.openSocket ?? ((u: string) => new WebSocket(u) as unknown as WebSocketLike);
  const startCapture = deps.startCapture ?? startMicPcm;
  const now = deps.now ?? Date.now;

  let active = false;
  let stopping = false;
  let generation = 0;
  let ws: WebSocketLike | null = null;
  let capture: PcmCapture | null = null;
  let ready = false;
  let pending: ArrayBuffer[] = [];
  let pendingSamples = 0;
  let utteranceStartedAt = 0;
  let tailTimer: ReturnType<typeof setTimeout> | null = null;
  let finalizeTimer: ReturnType<typeof setTimeout> | null = null;

  function clearTimers() {
    if (tailTimer) clearTimeout(tailTimer);
    if (finalizeTimer) clearTimeout(finalizeTimer);
    tailTimer = finalizeTimer = null;
  }

  function release() {
    capture?.stop();
    capture = null;
    setMicEnabled(false);
    const s = ws;
    ws = null;
    if (s) {
      s.onmessage = s.onclose = s.onerror = s.onopen = null;
      try {
        s.close();
      } catch {
        /* ignore */
      }
    }
    ready = false;
    pending = [];
    pendingSamples = 0;
  }

  function finish() {
    if (!active) return;
    active = false;
    stopping = false;
    clearTimers();
    release();
    cb.onSpeechActivity?.(false);
    cb.onPartial("");
    cb.onEnd?.();
  }

  function sendAudio(buf: ArrayBuffer, sampleRate: number) {
    if (!ws) return;
    if (ready) {
      ws.send(buf);
      return;
    }
    // Hold early audio (the first word) until the bridge is ready; bounded.
    pending.push(buf);
    pendingSamples += buf.byteLength / 2;
    while (pendingSamples > (sampleRate * PRE_READY_MAX_MS) / 1000 && pending.length) {
      pendingSamples -= (pending.shift() as ArrayBuffer).byteLength / 2;
    }
  }

  function onMessage(m: BridgeMsg, gen: number) {
    if (gen !== generation) return;
    switch (m.type) {
      case "ready": {
        ready = true;
        for (const b of pending) ws?.send(b);
        pending = [];
        pendingSamples = 0;
        break;
      }
      case "utterance":
        utteranceStartedAt = now();
        break;
      case "partial": {
        const full = [m.committed, m.provisional].filter(Boolean).join(" ");
        cb.onPartial(presentCeText(full));
        break;
      }
      case "final": {
        const text = presentCeText(m.text);
        const endedAt = now();
        if (text) {
          cb.onFinal({
            text,
            lang,
            startedAt: utteranceStartedAt || endedAt - m.durationMs,
            endedAt,
            engine: "ce-local",
          });
        }
        utteranceStartedAt = 0;
        cb.onPartial("");
        break;
      }
      case "activity":
        cb.onSpeechActivity?.(m.active);
        break;
      case "error": {
        if (m.code === "language-unsupported") {
          cb.onError("language-unsupported", languageUnsupportedMessage(lang, "Switch Talktual back to the browser recognizer for this language."), true);
        } else {
          cb.onError("provider", `Communication Engine: ${m.message}`, m.fatal);
        }
        if (m.fatal) finish();
        break;
      }
      case "end":
        finish();
        break;
      default:
        break;
    }
  }

  return {
    engine: "ce-local",
    route: "ce-local" as ProcessingRoute,
    get active() {
      return active;
    },

    async start() {
      if (active && !stopping) return;
      if (stopping && tailTimer) {
        // Pressed again during the release tail: keep streaming.
        stopping = false;
        clearTimers();
        return;
      }
      if (active) return; // finishing the previous push; the session retries on its end
      active = true;
      stopping = false;
      const gen = ++generation;
      let cap: PcmCapture;
      try {
        cap = await startCapture((chunk) => {
          if (gen === generation && capture) sendAudio(toPcm16(chunk), capture.sampleRate);
        });
      } catch (err) {
        if (gen !== generation) return;
        active = false;
        cb.onError(captureErrorCode(err), `Microphone unavailable: ${String(err)}`, true);
        throw err;
      }
      if (gen !== generation || !active) {
        cap.stop();
        return;
      }
      capture = cap;
      setMicEnabled(true);
      cb.onSpeechActivity?.(mode !== "continuous");
      let socket: WebSocketLike;
      try {
        socket = openSocket(url);
      } catch {
        cb.onError("network", `The Communication Engine bridge is not reachable at ${url}.`, true);
        finish();
        return;
      }
      ws = socket;
      socket.binaryType = "arraybuffer";
      socket.onopen = () => {
        if (gen !== generation) return;
        socket.send(
          JSON.stringify({
            type: "start",
            lang,
            sampleRate: cap.sampleRate,
            segmentation: mode === "continuous" ? "activity" : "continuous",
          })
        );
      };
      socket.onmessage = (e) => {
        if (typeof e.data !== "string") return;
        try {
          onMessage(JSON.parse(e.data) as BridgeMsg, gen);
        } catch {
          /* malformed message: ignore */
        }
      };
      socket.onerror = () => {
        if (gen !== generation) return;
        cb.onError("network", `The Communication Engine bridge is not reachable at ${url}. Start it and try again.`, true);
        finish();
      };
      socket.onclose = () => {
        if (gen === generation) finish();
      };
    },

    stop() {
      if (!active || stopping) return;
      stopping = true;
      const gen = generation;
      const flush = () => {
        tailTimer = null;
        if (gen !== generation) return;
        capture?.stop();
        capture = null;
        setMicEnabled(false);
        if (ws && ws.readyState === 1 && ready) ws.send(JSON.stringify({ type: "stop" }));
        else finish(); // never got going: nothing to finalize
      };
      if (mode === "continuous") flush();
      else tailTimer = setTimeout(flush, RELEASE_TAIL_MS);
      finalizeTimer = setTimeout(() => {
        finalizeTimer = null;
        if (gen === generation) finish();
      }, (mode === "continuous" ? 0 : RELEASE_TAIL_MS) + CE_FINALIZE_TIMEOUT_MS);
    },

    abort() {
      generation++;
      const wasActive = active;
      try {
        if (ws && ws.readyState === 1) ws.send(JSON.stringify({ type: "abort" }));
      } catch {
        /* ignore */
      }
      active = false;
      stopping = false;
      clearTimers();
      release();
      if (wasActive) cb.onSpeechActivity?.(false);
    },

    dispose() {
      this.abort();
    },
  };
}
