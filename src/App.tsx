import React, { useState, useEffect } from 'react';
import { Home } from './components/Home';
import { StartCall } from './components/StartCall';
import { JoinCall } from './components/JoinCall';
import { CallRoom } from './components/CallRoom';
import { Language } from './types';

type Screen = 'home' | 'start' | 'join' | 'call';

function AppContent() {
  const [currentScreen, setCurrentScreen] = useState<Screen>('home');
  const [language, setLanguage] = useState<Language | null>(null);
  const [roomCode, setRoomCode] = useState<string>('');
  const [userName, setUserName] = useState<string>('');

  const warmUpSpeechSynthesis = () => {
    if ('speechSynthesis' in window) {
      window.speechSynthesis.cancel();
      const utterance = new SpeechSynthesisUtterance('');
      utterance.volume = 0;
      window.speechSynthesis.speak(utterance);
    }
  };
  
  useEffect(() => {
    // Check if there's a ?join= params
    const params = new URLSearchParams(window.location.search);
    const joinCode = params.get('join');
    if (joinCode) {
      setRoomCode(joinCode.toUpperCase());
      setCurrentScreen('join');
    }
  }, []);

  const handleStartCall = (lang: Language, name: string) => {
    warmUpSpeechSynthesis();
    setLanguage(lang);
    setUserName(name);
    setCurrentScreen('start');
  };

  const handleJoinCall = (lang: Language, name: string) => {
    warmUpSpeechSynthesis();
    setLanguage(lang);
    setUserName(name);
    setCurrentScreen('join');
  };

  const handleJoinedRoom = (code: string, lang?: Language) => {
    warmUpSpeechSynthesis();
    if (lang) setLanguage(lang);
    setRoomCode(code);
    setCurrentScreen('call');
  };

  const handleEndCall = () => {
    setRoomCode('');
    setCurrentScreen('home');
    // Also remove ?join param if present
    if (window.history.pushState) {
      const newurl = window.location.protocol + "//" + window.location.host + window.location.pathname;
      window.history.pushState({path:newurl},'',newurl);
    }
  };

  if (currentScreen === 'home') {
    return <Home onStartCall={handleStartCall} onJoinCall={handleJoinCall} />;
  }
  
  if (currentScreen === 'start' && language) {
    return <StartCall language={language} onBack={() => setCurrentScreen('home')} onJoined={handleJoinedRoom} />;
  }

  if (currentScreen === 'join') {
    if (!language) {
      return <Home onStartCall={handleStartCall} onJoinCall={handleJoinCall} />;
    }
    return <JoinCall language={language} onBack={() => setCurrentScreen('home')} onJoined={(code, lang) => { setLanguage(lang); handleJoinedRoom(code); }} initialCode={roomCode} />;
  }

  if (currentScreen === 'call' && language) {
    return <CallRoom roomCode={roomCode} language={language} userName={userName} onEndCall={handleEndCall} />;
  }

  return null;
}

export default function App() {
  return <AppContent />;
}
