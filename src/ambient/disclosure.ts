// What ambient listening captures, processes, transmits and keeps — per
// processing mode. Shown before the first use on this device and on demand.
// Accurate to the implementation: no audio is recorded or stored by
// Talktual; the recognizer is the browser's (cloud unless on-device).

import type { Disclosure } from '../engine/privacy';
import type { ProcessingMode } from '../privacy/policy';

// v2 (Phase 8.5): the disclosure now names the speech-output processor.
export const AMBIENT_CONSENT_VERSION = 2;
export const ambientConsentKey = (route: string) => `talktual.ambient.consent.v${AMBIENT_CONSENT_VERSION}.${route}`;

export function ambientDisclosure(translator: string | null, mode: ProcessingMode, serverTts = false): Disclosure {
  const translation =
    translator === 'gemini' ? 'Google Gemini' : translator === 'deepl' ? 'DeepL' : translator === 'mymemory' ? 'MyMemory' : 'the translation service configured by this Talktual server';
  const recognizer =
    mode === 'local-only'
      ? 'Speech is recognised on this device only. If this device can’t do that for the chosen language, listening stays off.'
      : 'While listening is on, your browser’s speech-recognition service hears the room through the microphone. In Chrome and Edge that is a Google/Microsoft cloud service; in Safari, Apple’s.';
  const text =
    mode === 'local-only'
      ? 'Text is translated only on this device; anything it can’t translate is shown as heard.'
      : mode === 'hybrid'
        ? `Recognised text is translated on this device where possible, otherwise by ${translation}.`
        : `Recognised text is sent to ${translation} for translation.`;
  const speech =
    mode === 'local-only'
      ? 'When speech is on, translations are spoken by a voice on this device.'
      : serverTts
        ? 'When speech is on, the translated text is sent through the Talktual server to ElevenLabs to be spoken (text only, no audio of the room). If that fails, your device’s voice is used.'
        : 'When speech is on, translations are spoken by your browser’s voice (on some devices that voice is itself a cloud service).';
  return {
    route: 'browser-speech',
    title: 'Listening to the room',
    points: [
      'Live translate listens to people near you, not just to you. It only listens after you press Start, shows that it is listening the whole time, and stops the moment you press Pause or leave the screen.',
      recognizer,
      text,
      speech,
      'Talktual doesn’t record or store audio. The text stays on this screen until you clear it or leave, and is never saved.',
      'People nearby haven’t agreed to be translated. Use it where listening is appropriate, and respect requests to stop.',
    ],
  };
}
