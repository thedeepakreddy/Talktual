// Free speech-to-text via the browser's Web Speech API.
// Chrome/Edge only — no support in Safari or Firefox.
// Swap this file's internals later for Deepgram/Whisper streaming;
// keep the same function signature so nothing else needs to change.

export interface RecognitionHandle {
  stop: () => void;
}

export function startListening(
  lang: string,
  onResult: (text: string) => void,
  onError?: (error: string) => void
): RecognitionHandle | null {
  const SpeechRecognitionCtor =
    (window as any).SpeechRecognition || (window as any).webkitSpeechRecognition;

  if (!SpeechRecognitionCtor) {
    onError?.("Speech recognition not supported in this browser");
    return null;
  }

  const recognition = new SpeechRecognitionCtor();
  recognition.lang = lang; // e.g. "hu-HU", "fa-IR"
  recognition.continuous = true;
  recognition.interimResults = false;

  recognition.onresult = (event: any) => {
    const result = event.results[event.results.length - 1];
    if (result.isFinal) {
      onResult(result[0].transcript.trim());
    }
  };

  recognition.onerror = (event: any) => {
    onError?.(event.error);
  };

  // Auto-restart on silence timeout — Web Speech API stops itself after
  // a pause, which we don't want during push-to-talk holds.
  recognition.onend = () => {
    if ((recognition as any)._shouldRestart) {
      recognition.start();
    }
  };

  (recognition as any)._shouldRestart = true;
  recognition.start();

  return {
    stop: () => {
      (recognition as any)._shouldRestart = false;
      recognition.stop();
    },
  };
}
