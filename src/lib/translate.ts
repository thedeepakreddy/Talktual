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

  const pending = inFlight.get(k);
  if (pending) return pending;

  const request = (async () => {
    try {
      const res = await fetch(`${API_BASE}/api/translate`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ text: trimmed, from, to }),
        signal,
      });
      if (!res.ok) throw new Error(`translate failed: ${res.status}`);
      const data = await res.json();
      const out = (data.text as string) || trimmed;
      remember(k, out);
      return out;
    } catch (err) {
      if ((err as Error)?.name === "AbortError") throw err;
      console.error("translateText error:", err);
      // Never break the call over a translation failure — the untranslated
      // text still carries meaning to someone with partial fluency.
      return trimmed;
    } finally {
      inFlight.delete(k);
    }
  })();

  inFlight.set(k, request);
  return request;
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
