// CapabilityRegistry: what this browser and this server can do, and what the
// call can therefore offer. Pure functions — provider *selection* stays on the
// server, and none of this lives in the session state machine.

import type { ProcessingRoute } from './events';
import type { AsrEngine, ServerCapabilities } from './ports';

export interface BrowserCapabilities {
  webSpeech: boolean;
  getUserMedia: boolean;
  mediaRecorder: boolean;
  webAudio: boolean;
  speechSynthesis: boolean;
  dataChannel: boolean;
  secureContext: boolean;
}

export function detectBrowserCapabilities(w: Window & typeof globalThis = window): BrowserCapabilities {
  const anyW = w as unknown as Record<string, unknown>;
  return {
    webSpeech: Boolean(anyW.SpeechRecognition || anyW.webkitSpeechRecognition),
    getUserMedia: Boolean(w.navigator?.mediaDevices?.getUserMedia),
    mediaRecorder: typeof anyW.MediaRecorder === 'function',
    webAudio: Boolean(anyW.AudioContext || anyW.webkitAudioContext),
    speechSynthesis: 'speechSynthesis' in w,
    dataChannel: typeof anyW.RTCPeerConnection === 'function',
    secureContext: w.isSecureContext !== false,
  };
}

export type DegradationReason =
  | 'insecure-context'
  | 'no-speech-recognition'
  | 'no-capture'
  | 'no-server-stt'
  | 'no-audio-output'
  | 'no-p2p';

export interface ResolvedFeatures {
  /** Which recognizer the call will use, or null if the user cannot talk. */
  stt: AsrEngine | null;
  /** Where captured audio goes (what consent covers). */
  route: ProcessingRoute | null;
  canTalk: boolean;
  tts: 'server' | 'browser' | 'none';
  liveCaptions: boolean;
  p2p: boolean;
  reasons: DegradationReason[];
  /** False until server capabilities are known. */
  serverKnown: boolean;
}

export function resolveFeatures(browser: BrowserCapabilities, server: ServerCapabilities | null): ResolvedFeatures {
  const reasons: DegradationReason[] = [];
  let stt: AsrEngine | null = null;

  if (!browser.secureContext) reasons.push('insecure-context');
  if (browser.webSpeech && browser.secureContext) {
    stt = 'webspeech';
  } else if (browser.getUserMedia && browser.mediaRecorder && browser.secureContext) {
    // Server STT path — only usable if the server has it (unknown = optimistic
    // until capabilities load; the session re-resolves when they arrive).
    if (server === null || server.stt) stt = 'server-stt';
    else reasons.push('no-server-stt');
  } else {
    reasons.push(browser.getUserMedia ? 'no-speech-recognition' : 'no-capture');
  }

  const tts: ResolvedFeatures['tts'] = server?.tts && browser.webAudio ? 'server' : browser.speechSynthesis ? 'browser' : server?.tts ? 'server' : 'none';
  if (tts === 'none') reasons.push('no-audio-output');
  if (!browser.dataChannel) reasons.push('no-p2p');

  return {
    stt,
    route: stt === 'webspeech' ? 'browser-speech' : stt === 'server-stt' ? 'server-stt' : null,
    canTalk: stt !== null,
    tts,
    liveCaptions: Boolean(server?.liveCaptions),
    p2p: browser.dataChannel,
    reasons,
    serverKnown: server !== null,
  };
}

/** User-facing explanation for the most important degradation, if any. */
export function degradationMessage(f: ResolvedFeatures): string | null {
  if (f.reasons.includes('insecure-context')) {
    return 'The microphone needs a secure (https) connection. You can still hear the other person.';
  }
  if (f.reasons.includes('no-server-stt') || f.reasons.includes('no-speech-recognition') || f.reasons.includes('no-capture')) {
    return 'Speech recognition isn’t available in this browser on this server. Try Chrome, Edge or Safari — you can still hear the other person.';
  }
  return null;
}
