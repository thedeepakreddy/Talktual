import {StrictMode} from 'react';
import {createRoot} from 'react-dom/client';
import App from './App.tsx';
import './index.css';
import { installAudioUnlock } from './lib/audio';
import { preloadVoices } from './lib/textToSpeech';

// Translated speech starts long after the tap that started listening or a call.
// Browsers (iOS Safari above all) only play audio once the page was unlocked
// inside a real gesture: do it on every tap, and load the voice list early.
installAudioUnlock();
preloadVoices();

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
