// Single source of truth for where the signaling server and its API proxy live.

const RAW = import.meta.env.VITE_SIGNALING_URL || "ws://localhost:8080";

export const SIGNALING_URL = RAW.replace(/\/$/, "");

// "wss://host" -> "https://host", "ws://host" -> "http://host"
export const API_BASE = SIGNALING_URL.replace(/^ws/, "http");

/** How long after the user releases PTT we keep accepting trailing words. */
export const RELEASE_TAIL_MS = 450;

/** Max wait for the recognizer to finalize the last phrase before we force it. */
export const FINALIZE_TIMEOUT_MS = 1200;
