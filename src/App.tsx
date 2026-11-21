import { useState, useEffect, useCallback } from 'react';
import { Home } from './components/Home';
import { StartCall } from './components/StartCall';
import { JoinCall } from './components/JoinCall';
import { CallRoom } from './components/CallRoom';
import { Language } from './types';
import { unlockAudio } from './lib/audio';
import { prewarm } from './lib/translate';
import { normalizeRoomCode } from './lib/roomCode';

type Screen = 'home' | 'start' | 'join' | 'call';

export default function App() {
  const [currentScreen, setCurrentScreen] = useState<Screen>('home');
  const [language, setLanguage] = useState<Language | null>(null);
  const [roomCode, setRoomCode] = useState('');
  const [userName, setUserName] = useState('');

  // Mobile browsers only allow audio playback if the page has already played
  // some during a real user gesture. Every entry point below runs from a tap,
  // so this is the one chance to unlock before the first translation arrives.
  const primeAudio = () => {
    void unlockAudio();
    prewarm();
  };

  useEffect(() => {
    const joinCode = normalizeRoomCode(new URLSearchParams(window.location.search).get('join'));
    if (joinCode) {
      setRoomCode(joinCode);
      setCurrentScreen('join');
    }
  }, []);

  const handleStartCall = (lang: Language, name: string) => {
    primeAudio();
    setLanguage(lang);
    setUserName(name);
    setCurrentScreen('start');
  };

  const handleJoinCall = (lang: Language, name: string) => {
    primeAudio();
    setLanguage(lang);
    setUserName(name);
    setCurrentScreen('join');
  };

  const handleJoinedRoom = useCallback((code: string, lang?: Language) => {
    primeAudio();
    if (lang) setLanguage(lang);
    setRoomCode(code);
    setCurrentScreen('call');
  }, []);

  const handleEndCall = useCallback(() => {
    setRoomCode('');
    setCurrentScreen('home');
    if (window.history.pushState) {
      const url = window.location.protocol + '//' + window.location.host + window.location.pathname;
      window.history.pushState({ path: url }, '', url);
    }
  }, []);

  if (currentScreen === 'start' && language) {
    return (
      <StartCall
        language={language}
        onBack={() => setCurrentScreen('home')}
        onJoined={handleJoinedRoom}
      />
    );
  }

  if (currentScreen === 'join' && language) {
    return (
      <JoinCall
        language={language}
        onBack={() => setCurrentScreen('home')}
        onJoined={(code, lang) => handleJoinedRoom(code, lang)}
        initialCode={roomCode}
      />
    );
  }

  if (currentScreen === 'call' && language) {
    return (
      <CallRoom
        roomCode={roomCode}
        language={language}
        userName={userName}
        onLanguageChange={setLanguage}
        onEndCall={handleEndCall}
      />
    );
  }

  return <Home onStartCall={handleStartCall} onJoinCall={handleJoinCall} />;
}
