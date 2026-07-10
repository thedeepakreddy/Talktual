// Text-to-speech proxied through our signaling server to hide API keys.
// Falls back to free browser speechSynthesis if the request fails.

const SIGNALING_URL = import.meta.env.VITE_SIGNALING_URL || "ws://localhost:8080";
const API_BASE = SIGNALING_URL.replace(/^ws/, 'http');

export async function speak(
  text: string,
  lang: string,
  onEnd?: () => void
): Promise<void> {
  try {
    const res = await fetch(`${API_BASE}/api/tts`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ text }),
    });

    if (!res.ok) throw new Error(`TTS request failed: ${res.status}`);

    const audioBlob = await res.blob();
    const audioUrl = URL.createObjectURL(audioBlob);
    const audio = new Audio(audioUrl);

    audio.onended = () => {
      URL.revokeObjectURL(audioUrl);
      onEnd?.();
    };

    audio.onerror = () => {
      URL.revokeObjectURL(audioUrl);
      onEnd?.();
    };

    await audio.play();
  } catch (err) {
    console.error("ElevenLabs TTS failed, falling back to browser voice:", err);
    fallbackSpeak(text, lang, onEnd);
  }
}

function fallbackSpeak(text: string, lang: string, onEnd?: () => void) {
  if (!("speechSynthesis" in window)) {
    console.warn("speechSynthesis not supported in this browser");
    onEnd?.();
    return;
  }
  const utterance = new SpeechSynthesisUtterance(text);
  utterance.lang = lang;
  if (onEnd) utterance.onend = onEnd;
  speechSynthesis.speak(utterance);
}

export function preloadVoices(): Promise<any[]> {
  return Promise.resolve([]);
}
