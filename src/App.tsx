import { useState, useEffect, useCallback } from 'react';
import { MotionConfig } from 'motion/react';
import { Home } from './components/Home';
import { StartCall } from './components/StartCall';
import { JoinCall } from './components/JoinCall';
import { CallRoom } from './components/CallRoom';
import { Language, detectLanguage } from './types';
import { unlockAudio } from './lib/audio';
import { prewarm } from './lib/translate';
import { normalizeRoomCode } from './lib/roomCode';

type Screen = 'home' | 'start' | 'join' | 'call';

export default function App() {
  const [screen, setScreen] = useState<Screen>('home');
  const [language, setLanguage] = useState<Language>(() => detectLanguage());
  const [userName, setUserName] = useState('');
  const [roomCode, setRoomCode] = useState('');
  /** Arrived through an invite link: the code is known, skip the home screen. */
  const [invited, setInvited] = useState(false);

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
      setInvited(true);
      setScreen('join');
    }
  }, []);

  const clearInviteUrl = () => {
    if (window.history.pushState) {
      const url = window.location.protocol + '//' + window.location.host + window.location.pathname;
      window.history.pushState({ path: url }, '', url);
    }
  };

  const start = () => {
    primeAudio();
    setScreen('start');
  };
  const join = () => {
    primeAudio();
    setRoomCode('');
    setInvited(false);
    setScreen('join');
  };

  const enterCall = useCallback((code: string) => {
    primeAudio();
    setRoomCode(code);
    setScreen('call');
  }, []);

  const endCall = useCallback(() => {
    setRoomCode('');
    setInvited(false);
    setScreen('home');
    clearInviteUrl();
  }, []);

  const back = () => {
    setInvited(false);
    setScreen('home');
    clearInviteUrl();
  };

  return (
    <MotionConfig reducedMotion="user">
      {screen === 'start' && <StartCall language={language} onBack={back} onJoined={enterCall} />}
      {screen === 'join' && (
        <JoinCall
          language={language}
          onLanguageChange={setLanguage}
          userName={userName}
          onUserNameChange={setUserName}
          initialCode={roomCode}
          invited={invited}
          onBack={back}
          onJoined={(code) => enterCall(code)}
        />
      )}
      {screen === 'call' && (
        <CallRoom roomCode={roomCode} language={language} userName={userName} onLanguageChange={setLanguage} onEndCall={endCall} />
      )}
      {screen === 'home' && (
        <Home language={language} onLanguageChange={setLanguage} userName={userName} onUserNameChange={setUserName} onStart={start} onJoin={join} />
      )}
    </MotionConfig>
  );
}
