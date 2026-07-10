// Free text-to-speech via the browser's speechSynthesis API.
// Voice availability per language varies a lot by device/OS — test the
// specific language pairs you care about early.
// Swap this file's internals later for ElevenLabs/Azure; keep the same
// function signature so nothing else needs to change.

export function speak(
  text: string,
  lang: string,
  onEnd?: () => void
): void {
  if (!("speechSynthesis" in window)) {
    console.warn("speechSynthesis not supported in this browser");
    onEnd?.();
    return;
  }

  const utterance = new SpeechSynthesisUtterance(text);
  utterance.lang = lang; // e.g. "fa-IR", "hu-HU"

  const voices = speechSynthesis.getVoices();
  const match = voices.find((v) => v.lang === lang || v.lang.startsWith(lang.split("-")[0]));
  if (match) utterance.voice = match;

  if (onEnd) utterance.onend = onEnd;

  speechSynthesis.speak(utterance);
}

// Call this once on app load — voice list loads asynchronously on some browsers
export function preloadVoices(): Promise<SpeechSynthesisVoice[]> {
  return new Promise((resolve) => {
    const voices = speechSynthesis.getVoices();
    if (voices.length > 0) {
      resolve(voices);
      return;
    }
    speechSynthesis.onvoiceschanged = () => {
      resolve(speechSynthesis.getVoices());
    };
  });
}
