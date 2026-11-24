// Single source of truth for where the signaling server and its API proxy live.

const RAW = import.meta.env.VITE_SIGNALING_URL || "ws://localhost:8080";

export const SIGNALING_URL = RAW.replace(/\/$/, "");

// "wss://host" -> "https://host", "ws://host" -> "http://host"
export const API_BASE = SIGNALING_URL.replace(/^ws/, "http");

/** How long after the user releases PTT we keep accepting trailing words. */
export const RELEASE_TAIL_MS = 450;

/**
 * Speech recognizer selection. "ce" uses the Communication Engine through the
 * local Talktual ↔ CE bridge (communication-engine/adapters/talktual-bridge);
 * anything else keeps the browser recognizer (default).
 */
export const ASR_ENGINE: "ce" | "browser" = import.meta.env.VITE_ASR_ENGINE === "ce" ? "ce" : "browser";

/** Where the Talktual ↔ CE bridge listens (localhost by default). */
export const CE_BRIDGE_URL: string = (import.meta.env.VITE_CE_BRIDGE_URL || "ws://127.0.0.1:8091").replace(/\/$/, "");

/** Max wait for the recognizer to finalize the last phrase before we force it. */
export const FINALIZE_TIMEOUT_MS = 1200;
