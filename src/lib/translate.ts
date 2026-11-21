// Translation goes through our own proxy so the provider (and its key) is a
// server-side decision. The proxy picks the fastest configured provider and
// falls back to MyMemory, which needs no key at all.

import { API_BASE } from "./config";

export type TranslatorId = "gemini" | "deepl" | "mymemory";

export interface Capabilities {
  tts: boolean;
  stt: boolean;
  translator: TranslatorId;
  /** True when the translator is cheap/fast enough to run on interim text. */
  liveCaptions: boolean;
}

/**
 * Raised when no translation could be produced. Callers decide how to keep the
 * call going (e.g. send the original text, clearly marked) — the failure must
 * never be passed off as a successful translation.
 */
export class TranslationError extends Error {
  constructor(
    message: string,
    readonly reason: "http" | "timeout" | "network" | "malformed"
  ) {
    super(message);
    this.name = "TranslationError";
  }
}

/** Client-side budget; slightly above the server's provider timeout. */
export const TRANSLATE_TIMEOUT_MS = 10_000;

const cache = new Map<string, string>();
const inFlight = new Map<string, Promise<string>>();
const MAX_CACHE = 500;

function key(text: string, from: string, to: string) {
  return `${from}>${to}:${text}`;
}

function remember(k: string, value: string) {
  if (cache.size >= MAX_CACHE) {
    // Cheap FIFO eviction — order of insertion is Map's iteration order.
    const oldest = cache.keys().next().value;
    if (oldest !== undefined) cache.delete(oldest);
  }
  cache.set(k, value);
}

/**
 * Translates one piece of text. Resolves with the translation, or rejects with
 * a TranslationError (or the caller's AbortError). Only real translations are
 * cached, so a transient provider outage can't poison later turns.
 */
export async function translateText(
  text: string,
  fromLang: string,
  toLang: string,
  signal?: AbortSignal
): Promise<string> {
  const trimmed = text.trim();
  if (!trimmed) return "";

  const from = fromLang.split("-")[0];
  const to = toLang.split("-")[0];
  if (from === to) return trimmed;

  const k = key(trimmed, from, to);
  const cached = cache.get(k);
  if (cached !== undefined) return cached;

  // Callers with their own abort signal (live captions) don't share requests:
  // aborting one must not reject another caller's promise.
  const pending = signal ? undefined : inFlight.get(k);
  if (pending) return pending;

  const request = (async () => {
    // Manual combine: AbortSignal.any/timeout are too new for older iOS Safari.
    const controller = new AbortController();
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, TRANSLATE_TIMEOUT_MS);
    const onAbort = () => controller.abort();
    if (signal) {
      if (signal.aborted) controller.abort();
      else signal.addEventListener("abort", onAbort, { once: true });
    }
    try {
      return await fetchTranslation(k, trimmed, from, to, controller.signal, () => timedOut, signal);
    } finally {
      clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
    }
  })();

  if (!signal) {
    inFlight.set(k, request);
    request.then(
      () => inFlight.delete(k),
      () => inFlight.delete(k)
    );
  }
  return request;
}

async function fetchTranslation(
  k: string,
  trimmed: string,
  from: string,
  to: string,
  combined: AbortSignal,
  timedOut: () => boolean,
  callerSignal?: AbortSignal
): Promise<string> {
  let res: Response;
  try {
    res = await fetch(`${API_BASE}/api/translate`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ text: trimmed, from, to }),
      signal: combined,
    });
  } catch (err) {
    if (callerSignal?.aborted && !timedOut()) throw err; // superseded by the caller — not a failure
    if (timedOut()) throw new TranslationError("Translation timed out", "timeout");
    throw new TranslationError(`Translation network error: ${String(err)}`, "network");
  }
  if (!res.ok) throw new TranslationError(`Translation failed (${res.status})`, "http");
  let data: { text?: unknown; error?: unknown };
  try {
    data = await res.json();
  } catch {
    throw new TranslationError("Translation response was not JSON", "malformed");
  }
  if (data.error || typeof data.text !== "string" || !data.text.trim()) {
    throw new TranslationError("Translation response had no text", "malformed");
  }
  remember(k, data.text);
  return data.text;
}

let capsPromise: Promise<Capabilities> | null = null;

export function getCapabilities(): Promise<Capabilities> {
  if (!capsPromise) {
    capsPromise = fetch(`${API_BASE}/api/capabilities`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .catch((): Capabilities => ({
        tts: false,
        stt: false,
        translator: "mymemory",
        liveCaptions: false,
      }));
  }
  return capsPromise;
}

/** Warms DNS/TLS to the proxy so the first real request isn't paying for it. */
export function prewarm() {
  fetch(`${API_BASE}/api/capabilities`).catch(() => {});
}
