// High-quality text-to-speech via ElevenLabs API.
// Uses the eleven_multilingual_v2 model to automatically detect and speak the correct language.

const ELEVENLABS_API_KEY = import.meta.env.VITE_ELEVENLABS_API_KEY;
const DEFAULT_VOICE_ID = '21m00Tcm4TlvDq8ikWAM'; // Rachel (high quality, multilingual)

export async function speak(
  text: string,
  lang: string,
  onEnd?: () => void
): Promise<void> {
  if (!ELEVENLABS_API_KEY) {
    console.warn("VITE_ELEVENLABS_API_KEY is not set. TTS is disabled.");
    onEnd?.();
    return;
  }

  try {
    const response = await fetch(`https://api.elevenlabs.io/v1/text-to-speech/${DEFAULT_VOICE_ID}`, {
      method: 'POST',
      headers: {
        'Accept': 'audio/mpeg',
        'Content-Type': 'application/json',
        'xi-api-key': ELEVENLABS_API_KEY
      },
      body: JSON.stringify({
        text,
        model_id: 'eleven_multilingual_v2',
        voice_settings: {
          stability: 0.5,
          similarity_boost: 0.75
        }
      })
    });

    if (!response.ok) {
      console.error("ElevenLabs TTS error:", await response.text());
      onEnd?.();
      return;
    }

    const blob = await response.blob();
    const url = URL.createObjectURL(blob);
    const audio = new Audio(url);
    
    audio.onended = () => {
      URL.revokeObjectURL(url);
      onEnd?.();
    };
    
    audio.onerror = () => {
      URL.revokeObjectURL(url);
      onEnd?.();
    };

    audio.play().catch(e => {
      console.error("Audio playback failed:", e);
      URL.revokeObjectURL(url);
      onEnd?.();
    });
  } catch (err) {
    console.error("TTS fetch error:", err);
    onEnd?.();
  }
}

// Preloading voices is no longer strictly necessary since we fetch from API,
// but we keep the signature for compatibility with CallRoom.tsx
export function preloadVoices(): Promise<any[]> {
  return Promise.resolve([]);
}
