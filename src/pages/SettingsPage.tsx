import { useEffect, useState } from 'react';
import type { Language } from '../types';
import { Page } from '../components/shell/AppShell';
import { Button, Row, Section, Segmented } from '../components/ui';
import { LanguagePicker } from '../components/LanguagePicker';
import { ConsentSheet } from '../components/ConsentSheet';
import { getThemePref, setThemePref, type ThemePref } from '../theme';
import { getModePref, setModePref, type ModePref } from '../app/prefs';
import { ambientConsentKey, ambientDisclosure } from '../ambient/disclosure';
import { clearTranslationCache, getCapabilities } from '../lib/translate';
import { micPermission } from '../lib/audio';
import type { Disclosure } from '../engine/privacy';

const MODE_HINT: Record<ModePref, string> = {
  cloud: 'Speech and text go to recognition, translation and voice services. This device is used when they can’t be reached.',
  hybrid: 'This device handles what it can; cloud services only for the rest.',
  'local-only': 'Nothing is sent to recognition, translation or voice services. Features this device can’t do on its own are unavailable — never silently sent to the cloud.',
};

const CALL_KEYS = ['talktual.consent.v1.browser-speech', 'talktual.consent.v1.server-stt', 'talktual.consent.v1.ce-local'];
const read = (k: string) => {
  try {
    return localStorage.getItem(k) === 'granted';
  } catch {
    return false;
  }
};
const forget = (keys: string[]) => {
  try {
    keys.forEach((k) => localStorage.removeItem(k));
  } catch {
    /* nothing stored */
  }
};

/** Settings & Privacy, organized by task. Every status shown is read live, not assumed. */
export function SettingsPage({ language, onLanguageChange }: { language: Language; onLanguageChange: (l: Language) => void }) {
  const [theme, setTheme] = useState<ThemePref>(() => getThemePref());
  const [mode, setMode] = useState<ModePref>(() => getModePref());
  const [picker, setPicker] = useState(false);
  const [airConsent, setAirConsent] = useState(() => read(ambientConsentKey('browser-speech')));
  const [callConsent, setCallConsent] = useState(() => CALL_KEYS.some(read));
  const [mic, setMic] = useState<string>('…');
  const [disclosure, setDisclosure] = useState<Disclosure | null>(null);
  const [cleared, setCleared] = useState(false);
  const [translator, setTranslator] = useState<string | null>(null);
  const [serverTts, setServerTts] = useState(false);

  useEffect(() => {
    micPermission()
      .then((p) => setMic(p === 'granted' ? 'Allowed' : p === 'denied' ? 'Blocked — change it in your browser’s site settings' : p === 'prompt' ? 'Asked when you first listen' : 'Unknown in this browser'))
      .catch(() => setMic('Unknown in this browser'));
    getCapabilities()
      .then((c) => {
        setTranslator(c.translator);
        setServerTts(c.tts);
      })
      .catch(() => {});
  }, []);

  return (
    <Page eyebrow="Preferences" title="Settings" description="Language, appearance, and how your speech is processed.">
      <Section title="Language">
        <Row label="Your language" value={`${language.flag} ${language.native}`} onClick={() => setPicker(true)} ariaLabel={`Your language: ${language.native}. Change`} />
      </Section>

      <Section title="Appearance">
        <div className="py-3">
          <Segmented<ThemePref>
            label="Theme"
            value={theme}
            onChange={(t) => {
              setTheme(t);
              setThemePref(t);
            }}
            options={[
              { value: 'system', label: 'System' },
              { value: 'light', label: 'Light' },
              { value: 'dark', label: 'Dark' },
            ]}
          />
          <p className="mt-2 text-label text-fg-muted">Text size and reduced motion follow your device settings.</p>
        </div>
      </Section>

      <Section title="Processing">
        <div className="py-3">
          <Segmented<ModePref>
            label="Processing mode"
            value={mode}
            onChange={(m) => {
              setMode(m);
              setModePref(m);
              clearTranslationCache();
            }}
            options={[
              { value: 'cloud', label: 'Cloud' },
              { value: 'hybrid', label: 'Hybrid' },
              { value: 'local-only', label: 'On-device' },
            ]}
          />
          <p className="mt-2 text-label text-fg-muted">{MODE_HINT[mode]}</p>
          <p className="mt-2 text-label text-fg-muted">Applies to the next AIR session or call. You can also change it during a session.</p>
        </div>
      </Section>

      <Section title="Privacy">
        <div className="flex min-h-12 items-center justify-between gap-4 py-2.5">
          <span className="text-body-lg font-[520]">Microphone</span>
          <span className="text-right text-label text-fg-muted" data-testid="mic-permission">
            {mic}
          </span>
        </div>
        <div className="flex min-h-12 items-center justify-between gap-4 py-2.5">
          <div>
            <p className="text-body-lg font-[520]">AIR listening permission</p>
            <p className="text-label text-fg-muted">{airConsent ? 'Given on this device' : 'Asked before AIR listens'}</p>
          </div>
          {airConsent && (
            <Button
              size="sm"
              variant="tertiary"
              onClick={() => {
                forget([ambientConsentKey('browser-speech')]);
                setAirConsent(false);
              }}
            >
              Forget
            </Button>
          )}
        </div>
        <div className="flex min-h-12 items-center justify-between gap-4 py-2.5">
          <div>
            <p className="text-body-lg font-[520]">Call microphone permission</p>
            <p className="text-label text-fg-muted">{callConsent ? 'Given on this device' : 'Asked before your first call'}</p>
          </div>
          {callConsent && (
            <Button
              size="sm"
              variant="tertiary"
              onClick={() => {
                forget(CALL_KEYS);
                setCallConsent(false);
              }}
            >
              Forget
            </Button>
          )}
        </div>
        <Row label="What AIR captures and where it goes" onClick={() => setDisclosure(ambientDisclosure(translator, mode, serverTts))} />
        <div className="py-3">
          <p className="text-body-lg font-[520]">What Talktual keeps</p>
          <p className="mt-1 text-label text-fg-muted">
            No audio, ever. Text stays on screen until you clear it or leave and is never saved. By default the server holds translated text for about two minutes to avoid repeat requests, then drops it.
          </p>
          <Button
            size="sm"
            variant="ghost"
            className="-ml-3 mt-2 text-danger"
            onClick={() => {
              clearTranslationCache();
              setCleared(true);
            }}
          >
            {cleared ? 'Cleared from this device' : 'Clear cached translations'}
          </Button>
        </div>
      </Section>

      <Section title="About">
        <p className="py-3 text-label text-fg-muted">Talktual by AskDeepakAI. Live translation (AIR) listens to people near you: use it where listening is appropriate.</p>
      </Section>

      <LanguagePicker open={picker} title="Your language" selected={language} onSelect={onLanguageChange} onClose={() => setPicker(false)} />
      <ConsentSheet open={disclosure !== null} disclosure={disclosure} granted onAllow={() => setDisclosure(null)} onDecline={() => setDisclosure(null)} />
    </Page>
  );
}
