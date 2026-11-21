// VoiceMirror disclosure. Separate purpose from microphone/speech consent:
// sharing derived voice characteristics with the other participant.
import type { Disclosure } from '../engine/privacy';

export const VOICEMIRROR_DISCLOSURE: Disclosure = {
  // `route` is required by the shared sheet; VoiceMirror audio never leaves the device.
  route: 'browser-speech',
  title: 'Share your voice style?',
  points: [
    'VoiceMirror makes the translated voice the other person hears closer to yours in pitch level and speaking speed. It does not copy or clone your voice.',
    'While you talk, this device measures your pitch and pace. No audio is recorded or uploaded for this — only three coarse settings are sent to the other person.',
    'Nothing is stored. Turn it off at any time and the other person’s app deletes your settings immediately; everything is deleted when the call ends.',
    'Experimental feature.',
  ],
};
