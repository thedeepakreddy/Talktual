// Browser VoiceTapPort: a short-lived analysis stream from the call's shared
// microphone. Audio stays in this tab; only the analyzer sees it.
//
// On Web Speech browsers the recognizer owns its own mic internally, so this
// opens the shared getUserMedia stream in parallel (tracks enabled only while
// the tap runs). Uses ScriptProcessorNode for broad support (Safari included);
// the per-chunk work is tiny (see PHASE3_EVALUATION_REPORT resources).

import { audioContext, getMic, setMicEnabled } from '../lib/audio';
import type { VoiceTapPort } from './controller.ts';

export function createBrowserVoiceTap(): VoiceTapPort {
  return {
    async start(onChunk) {
      const stream = await getMic();
      setMicEnabled(true);
      const ac = audioContext();
      if (ac.state === 'suspended') await ac.resume().catch(() => {});
      const source = ac.createMediaStreamSource(stream);
      const node = ac.createScriptProcessor(2048, 1, 1);
      const sink = ac.createGain();
      sink.gain.value = 0; // keep the node pulling without making any sound
      node.onaudioprocess = (e) => onChunk(new Float32Array(e.inputBuffer.getChannelData(0)), ac.sampleRate);
      source.connect(node);
      node.connect(sink);
      sink.connect(ac.destination);
      let stopped = false;
      return () => {
        if (stopped) return;
        stopped = true;
        node.onaudioprocess = null;
        try {
          source.disconnect();
          node.disconnect();
          sink.disconnect();
        } catch {
          /* already disconnected */
        }
        setMicEnabled(false);
      };
    },
  };
}
