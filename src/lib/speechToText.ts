// High-quality speech-to-text via ElevenLabs Scribe API.
// Captures audio using MediaRecorder during the push-to-talk hold,
// then uploads the batch to ElevenLabs upon release.

const ELEVENLABS_API_KEY = import.meta.env.VITE_ELEVENLABS_API_KEY;

export interface RecognitionHandle {
  stop: () => void;
}

export function startListening(
  lang: string,
  onResult: (text: string) => void,
  onError?: (error: string) => void
): RecognitionHandle | null {
  if (!ELEVENLABS_API_KEY) {
    onError?.("VITE_ELEVENLABS_API_KEY is missing. STT disabled.");
    return null;
  }

  let mediaRecorder: MediaRecorder | null = null;
  const audioChunks: Blob[] = [];

  navigator.mediaDevices.getUserMedia({ audio: true })
    .then(stream => {
      mediaRecorder = new MediaRecorder(stream);
      
      mediaRecorder.ondataavailable = (event) => {
        if (event.data.size > 0) {
          audioChunks.push(event.data);
        }
      };

      mediaRecorder.onstop = async () => {
        // Stop microphone tracks to release the hardware
        stream.getTracks().forEach(track => track.stop());

        const audioBlob = new Blob(audioChunks, { type: 'audio/webm' });
        
        try {
          const formData = new FormData();
          formData.append('file', audioBlob, 'recording.webm');
          formData.append('model_id', 'scribe_v1');
          
          const response = await fetch('https://api.elevenlabs.io/v1/speech-to-text', {
            method: 'POST',
            headers: {
              'xi-api-key': ELEVENLABS_API_KEY
            },
            body: formData
          });

          if (!response.ok) {
            const errorText = await response.text();
            console.error("ElevenLabs STT error:", errorText);
            onError?.("Transcription failed");
            return;
          }

          const data = await response.json();
          if (data.text) {
            onResult(data.text.trim());
          }
        } catch (err) {
          console.error("STT fetch error:", err);
          onError?.(String(err));
        }
      };

      mediaRecorder.start();
    })
    .catch(err => {
      console.error("Microphone access denied:", err);
      onError?.("Microphone access denied");
    });

  return {
    stop: () => {
      if (mediaRecorder && mediaRecorder.state !== "inactive") {
        mediaRecorder.stop();
      }
    }
  };
}
