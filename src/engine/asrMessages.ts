import { languageName } from '../types';

/**
 * Shown when the browser's speech recognizer refuses a language. Safari reports
 * this as "service-not-allowed", which used to be shown as a blocked microphone.
 */
export function languageUnsupportedMessage(lang: string, tail: string): string {
  const name = languageName(lang);
  const ua = typeof navigator !== 'undefined' ? navigator.userAgent : '';
  const apple = /iPhone|iPad|iPod|Macintosh/.test(ua) && !/Chrome|CriOS|Android/.test(ua);
  const fix = apple
    ? ` On iPhone or iPad, turn on Dictation and add a ${name} keyboard (Settings › General › Keyboard), then try again.`
    : ' Try a different browser, or check the language is installed for speech in your device settings.';
  return `This browser can’t recognise ${name} speech, so your microphone wasn’t used.${fix} ${tail}`;
}
