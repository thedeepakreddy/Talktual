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

/** VoiceMirror is a Phase 3 prototype: off unless the build sets VITE_VOICEMIRROR=1. */
export const VOICEMIRROR_FLAG = import.meta.env.VITE_VOICEMIRROR === '1';

/**
 * Phase 4 conversation layer: ON by default. VITE_CONVERSATION_LAYER=0 (or
 * "off") at build time rolls back to Phase 3 behaviour without code changes.
 */
export const CONVERSATION_FLAG = !['0', 'off', 'false'].includes(String(import.meta.env.VITE_CONVERSATION_LAYER ?? '').toLowerCase());

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
    voiceTap: VOICEMIRROR_FLAG && Boolean(navigator.mediaDevices?.getUserMedia) ? createBrowserVoiceTap() : undefined,
  };
}
