// Production wiring: the real adapters behind the engine's ports.

import { connectCall } from '../lib/webrtc';
import { createAsr } from '../lib/speechToText';
import { getCapabilities, translateSegment, translateText } from '../lib/translate';
import { playbackController } from '../lib/textToSpeech';
import { captureAdapter, unlockAudio } from '../lib/audio';
import { SIGNALING_URL } from '../lib/config';
import { detectBrowserCapabilities } from './capabilities';
import type { SessionDeps } from './CommunicationSession';
import type { StorageLike } from './ports';
import { createBrowserVoiceTap } from '../voicemirror/browserTap';
import { clearTranslationCache } from '../lib/translate';
import { BrowserTranslatorRuntime, LocalVoiceRuntime, OnDeviceAsrRuntime } from '../local/platform';

/** VoiceMirror is a Phase 3 prototype: off unless the build sets VITE_VOICEMIRROR=1. */
export const VOICEMIRROR_FLAG = import.meta.env.VITE_VOICEMIRROR === '1';

/**
 * Phase 4 conversation layer: ON by default. VITE_CONVERSATION_LAYER=0 (or
 * "off") at build time rolls back to Phase 3 behaviour without code changes.
 */
export const CONVERSATION_FLAG = !['0', 'off', 'false'].includes(String(import.meta.env.VITE_CONVERSATION_LAYER ?? '').toLowerCase());

/**
 * Phase 5 communication intelligence: ON by default (on-device rules, no extra
 * provider calls). VITE_COMMUNICATION_INTELLIGENCE=0 rolls back to Phase 4.
 */
export const INTELLIGENCE_FLAG = !['0', 'off', 'false'].includes(String(import.meta.env.VITE_COMMUNICATION_INTELLIGENCE ?? '').toLowerCase());

/** Phase 6 processing modes + on-device runtimes: ON by default. VITE_PROCESSING_MODES=0 rolls back to Phase 5. */
export const PROCESSING_FLAG = !['0', 'off', 'false'].includes(String(import.meta.env.VITE_PROCESSING_MODES ?? '').toLowerCase());
/** On-device speech recognition probe: OFF by default (crashed the tab on the test hardware). */
export const ONDEVICE_ASR_FLAG = import.meta.env.VITE_ONDEVICE_ASR === '1';

function browserStorage(): StorageLike | null {
  try {
    const s = window.localStorage;
    const probe = '__talktual_probe__';
    s.setItem(probe, '1');
    s.removeItem(probe);
    return s;
  } catch {
    return null; // private mode / blocked storage: consent lasts for the page
  }
}

export function createDefaultDeps(): SessionDeps {
  return {
    connect: connectCall,
    signalingUrl: SIGNALING_URL,
    createAsr,
    translate: translateSegment,
    translateCaption: translateText,
    playback: playbackController(),
    capture: captureAdapter,
    getServerCapabilities: getCapabilities,
    browser: detectBrowserCapabilities(),
    storage: browserStorage(),
    onUserGesture: () => void unlockAudio(),
    conversation: CONVERSATION_FLAG,
    intelligence: INTELLIGENCE_FLAG,
    processing: PROCESSING_FLAG
      ? {
          translators: [
            new BrowserTranslatorRuntime((globalThis as { Translator?: never }).Translator, [
              ['en', 'es'], ['es', 'en'], ['en', 'fr'], ['fr', 'en'], ['en', 'de'], ['de', 'en'], ['en', 'pt'], ['pt', 'en'], ['en', 'it'], ['it', 'en'], ['es', 'fr'], ['fr', 'es'], ['hu', 'en'], ['en', 'hu'],
            ]),
          ],
          voices: [new LocalVoiceRuntime(typeof speechSynthesis !== 'undefined' ? () => speechSynthesis.getVoices() : undefined)],
          asr: [
            new OnDeviceAsrRuntime(
              (window as unknown as { SpeechRecognition?: never; webkitSpeechRecognition?: never }).SpeechRecognition ?? (window as unknown as { webkitSpeechRecognition?: never }).webkitSpeechRecognition,
              ONDEVICE_ASR_FLAG,
              ['en-US', 'es-ES', 'fr-FR', 'de-DE', 'pt-BR', 'it-IT']
            ),
          ],
        }
      : undefined,
    clearCaches: clearTranslationCache,
    voiceTap: VOICEMIRROR_FLAG && Boolean(navigator.mediaDevices?.getUserMedia) ? createBrowserVoiceTap() : undefined,
  };
}
