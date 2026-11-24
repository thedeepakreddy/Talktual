import { useState, useEffect, useCallback } from 'react';
import { MotionConfig } from 'motion/react';
import { StartCall } from './components/StartCall';
import { JoinCall } from './components/JoinCall';
import { CallRoom } from './components/CallRoom';
import { AmbientScreen } from './components/ambient/AmbientScreen';
import { AppShell } from './components/shell/AppShell';
import { HomePage } from './pages/HomePage';
import { ConnectPage } from './pages/ConnectPage';
import { LensPage } from './pages/LensPage';
import { RewindPage } from './pages/RewindPage';
import { VoicePage } from './pages/VoicePage';
import { SettingsPage } from './pages/SettingsPage';
import { EnginePage } from './pages/EnginePage';
import { Language, detectLanguage, findLanguage } from './types';
import { unlockAudio } from './lib/audio';
import { prewarm } from './lib/translate';
import { normalizeRoomCode } from './lib/roomCode';
import { useHashRoute } from './app/routes';

/** Within Connect: the entry page, the invite (start) screen, joining, or the call itself. */
type ConnectStep = 'entry' | 'start' | 'join' | 'call';

const defaultAirFrom = (you: Language) => findLanguage(you.code === 'hu' ? 'en-US' : 'hu-HU')!;

export default function App() {
  const [route, go] = useHashRoute();
  const [step, setStep] = useState<ConnectStep>('entry');
  const [language, setLanguage] = useState<Language>(() => detectLanguage());
  const [airFrom, setAirFrom] = useState<Language>(() => defaultAirFrom(detectLanguage()));
  const [airAutostart, setAirAutostart] = useState(false);
  const [userName, setUserName] = useState('');
  const [roomCode, setRoomCode] = useState('');
  /** Arrived through an invite link: the code is known; ask only for name and language. */
  const [invited, setInvited] = useState(false);

  // Mobile browsers only allow audio playback if the page has already played
  // some during a real user gesture. Every entry point below runs from a tap.
  const primeAudio = () => {
    void unlockAudio();
    prewarm();
  };

  useEffect(() => {
    const joinCode = normalizeRoomCode(new URLSearchParams(window.location.search).get('join'));
    if (joinCode) {
      setRoomCode(joinCode);
      setInvited(true);
      setStep('join');
      go('connect');
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // AIR and "They speak" can't be the same language.
  const changeLanguage = (l: Language) => {
    setLanguage(l);
    if (l.code === airFrom.code) setAirFrom(defaultAirFrom(l));
  };

  const clearInviteUrl = () => {
    if (window.location.search && window.history.replaceState) {
      window.history.replaceState(null, '', window.location.pathname + window.location.hash);
    }
  };

  const enterCall = useCallback((code: string) => {
    primeAudio();
    setRoomCode(code);
    setStep('call');
  }, []);

  const leaveConnect = useCallback(() => {
    setRoomCode('');
    setInvited(false);
    setStep('entry');
    clearInviteUrl();
  }, []);

  // Navigating away from Connect never leaves a call running in the background.
  useEffect(() => {
    if (route !== 'connect' && step !== 'entry') leaveConnect();
  }, [route, step, leaveConnect]);

  const inSession = route === 'air' || (route === 'connect' && step !== 'entry');

  return (
    <MotionConfig reducedMotion="user">
      <AppShell route={route} go={go} showNav={!inSession}>
        {route === 'home' && (
          <HomePage
            language={language}
            airFrom={airFrom}
            onLanguageChange={changeLanguage}
            onAirFromChange={setAirFrom}
            go={go}
            onStartAir={() => {
              primeAudio();
              setAirAutostart(true);
              go('air');
            }}
          />
        )}
        {route === 'air' && (
          <AmbientScreen
            language={language}
            onLanguageChange={changeLanguage}
            from={airFrom}
            onFromChange={setAirFrom}
            autostart={airAutostart}
            onExit={() => {
              setAirAutostart(false);
              go('home');
            }}
          />
        )}
        {route === 'connect' && step === 'entry' && (
          <ConnectPage
            language={language}
            onLanguageChange={changeLanguage}
            userName={userName}
            onUserNameChange={setUserName}
            onStart={() => {
              primeAudio();
              setStep('start');
            }}
            onJoin={() => {
              primeAudio();
              setRoomCode('');
              setInvited(false);
              setStep('join');
            }}
          />
        )}
        {route === 'connect' && step === 'start' && <StartCall language={language} onBack={leaveConnect} onJoined={enterCall} />}
        {route === 'connect' && step === 'join' && (
          <JoinCall
            language={language}
            onLanguageChange={changeLanguage}
            userName={userName}
            onUserNameChange={setUserName}
            initialCode={roomCode}
            invited={invited}
            onBack={leaveConnect}
            onJoined={(code) => enterCall(code)}
          />
        )}
        {route === 'connect' && step === 'call' && (
          <CallRoom roomCode={roomCode} language={language} userName={userName} onLanguageChange={changeLanguage} onEndCall={leaveConnect} />
        )}
        {route === 'lens' && <LensPage go={go} />}
        {route === 'rewind' && <RewindPage go={go} />}
        {route === 'voice' && <VoicePage language={language} />}
        {route === 'settings' && <SettingsPage language={language} onLanguageChange={changeLanguage} />}
        {route === 'engine' && <EnginePage language={language} />}
      </AppShell>
    </MotionConfig>
  );
}
