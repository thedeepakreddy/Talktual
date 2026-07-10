// Speech-to-text proxied through our signaling server to hide API keys.

const SIGNALING_URL = import.meta.env.VITE_SIGNALING_URL || "ws://localhost:8080";
const API_BASE = SIGNALING_URL.replace(/^ws/, 'http');

export interface RecognitionHandle {
  stop: () => void;
}

export function startListening(
  lang: string,
  onResult: (text: string) => void,
  onError?: (error: string) => void
): RecognitionHandle | null {
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
          
          const response = await fetch(`${API_BASE}/api/stt`, {
            method: 'POST',
            body: formData
          });

          if (!response.ok) {
            const errorText = await response.text();
            console.error("STT Proxy failed:", errorText);
            alert(`STT Error: The server returned ${response.status}. Make sure ELEVENLABS_API_KEY is set in your backend. Details: ${errorText}`);
            onError?.("Transcription failed");
            return;
          }

          const data = await response.json();
          if (data.text) {
            onResult(data.text.trim());
          } else {
            alert("STT Error: ElevenLabs returned no text.");
            onError?.("No text returned");
          }
        } catch (err) {
          console.error("STT fetch error:", err);
          alert(`STT Network Error: ${String(err)}. Is the backend server running?`);
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
