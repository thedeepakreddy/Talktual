// PrivacyPolicyController: listening modes, consent and disclosure.
//
// Rules enforced by the session through this controller:
//   1. No microphone access and no speech recognition before the user has
//      consented to the processing route that will receive the audio.
//   2. Push-to-talk (default): capture only while the button is held (+ tail).
//   3. Continuous: only after an explicit opt-in, with a persistent indicator;
//      one action stops it. Never persisted across calls.
// Consent is remembered per route and per disclosure version, so changing what
// we disclose re-asks.

import type { ListeningMode, ProcessingRoute } from './events';
import type { StorageLike } from './ports';

export const CONSENT_VERSION = 1;
const key = (route: ProcessingRoute) => `talktual.consent.v${CONSENT_VERSION}.${route}`;

export interface Disclosure {
  route: ProcessingRoute;
  title: string;
  points: string[];
}

export function disclosureFor(route: ProcessingRoute, translator: string | null): Disclosure {
  const translation =
    translator === 'gemini'
      ? 'Google Gemini'
      : translator === 'deepl'
        ? 'DeepL'
        : translator === 'mymemory'
          ? 'MyMemory'
          : 'the translation service configured by this Talktual server';
  const speech =
    route === 'browser-speech'
      ? 'Your browser’s speech-recognition service turns your voice into text. In Chrome and Edge that is a Google/Microsoft cloud service; in Safari, Apple’s. Audio is sent to it only while you are talking.'
      : 'Your voice is recorded only while you hold the button, then sent through the Talktual server to ElevenLabs for transcription.';
  return {
    route,
    title: 'Before you talk',
    points: [
      speech,
      `The text is translated by ${translation} and spoken to the other person.`,
      'Talktual doesn’t store your audio or transcripts. Push-to-talk is the default; hands-free listening is optional and always shown on screen.',
    ],
  };
}

function safeStorage(storage: StorageLike | null): StorageLike | null {
  return storage;
}

export class PrivacyPolicyController {
  private _mode: ListeningMode = 'push-to-talk';
  private memory = new Set<ProcessingRoute>();

  constructor(private readonly storage: StorageLike | null) {}

  get mode(): ListeningMode {
    return this._mode;
  }

  setMode(mode: ListeningMode): void {
    this._mode = mode;
  }

  hasConsent(route: ProcessingRoute | null): boolean {
    if (!route) return false;
    if (this.memory.has(route)) return true;
    try {
      return safeStorage(this.storage)?.getItem(key(route)) === 'granted';
    } catch {
      return false;
    }
  }

  grant(route: ProcessingRoute): void {
    this.memory.add(route);
    try {
      this.storage?.setItem(key(route), 'granted');
    } catch {
      /* storage unavailable: consent lasts for this page */
    }
  }

  revoke(route: ProcessingRoute): void {
    this.memory.delete(route);
    try {
      this.storage?.removeItem(key(route));
    } catch {
      /* ignore */
    }
  }

  /** The single gate every capture path goes through. */
  mayCapture(route: ProcessingRoute | null): boolean {
    return this.hasConsent(route);
  }
}
