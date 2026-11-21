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
  };
}
