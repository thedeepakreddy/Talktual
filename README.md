Talktual is a real-time, peer-to-peer voice translation application designed to seamlessly bridge language barriers. It allows two users to connect via a secure WebRTC data channel (established through a QR code handshake) and communicate in their native languages using a push-to-talk interface.

The application records voice input, transcribes it, translates the text, and synthesizes the translated voice on the receiver's device, all in near real-time.

System Architecture
The project is divided into a lightweight static frontend and a secure Node.js backend.

Frontend (React + Vite)
WebRTC Data Channels: Establishes a direct peer-to-peer connection for low-latency transmission of translated text strings without relying on a central server for message routing.
MediaStream API: Captures user audio via the microphone using MediaRecorder during push-to-talk interactions.
Graceful Degradation: Integrates a fallback mechanism that shifts from high-fidelity ElevenLabs TTS to the browser's native SpeechSynthesis API if network requests drop, ensuring a call never goes silent.
QR Code Handshake: Utilizes react-qr-code and react-qr-reader to facilitate instant session joining without manual link sharing.
Backend (Node.js + Express + WebSockets)
Signaling Server: A custom WebSocket server that routes SDP offers, answers, and ICE candidates to negotiate the initial WebRTC connection between two peers.
API Proxy: An Express server handles POST /api/tts and POST /api/stt routes. This architecture strictly proxies requests to the ElevenLabs REST APIs, ensuring the ELEVENLABS_API_KEY remains securely hidden on the server and is never exposed to the client browser.
Multipart Data Handling: Uses multer to securely process incoming audio/webm blobs from the frontend before forwarding them to the upstream Scribe STT model.
External Services
ElevenLabs (TTS/STT): Leverages eleven_multilingual_v2 for highly realistic text-to-speech generation and scribe_v1 for accurate, punctuated speech-to-text transcription.
MyMemory Translation API: Handles language conversion via a CORS-enabled REST endpoint.
Getting Started
Prerequisites
Node.js (v18+)
An ElevenLabs API Key
Backend Setup (Signaling Server)
Navigate to the backend directory:
bash

cd signaling-server
Install dependencies:
bash

npm install
Create a .env file and add your ElevenLabs API Key:
env

ELEVENLABS_API_KEY=your_api_key_here
Start the server:
bash

npm start
The server will run on http://localhost:8080.
Frontend Setup (Vite)
Navigate to the project root:
bash

cd ..
Install dependencies:
bash

npm install
Create a .env.local file and configure your signaling URL:
env

VITE_SIGNALING_URL=ws://localhost:8080
VITE_MYMEMORY_EMAIL=your_email@example.com
(Note: The email is optional but raises your daily translation limit).
Start the development server:
bash

npm run dev
Deployment
Frontend: Configured to be deployed as a Static Site (e.g., Render, Vercel). Environment variables are injected at build time.
Backend: Configured to be deployed as a Node Web Service. It exposes both the WebSocket signaling endpoint and the HTTP API proxies on a single port.
