import { useEffect, useRef, useState } from 'react';
import { Play, Square } from '../components/icons';
import type { Language } from '../types';
import { Page } from '../components/shell/AppShell';
import { Button, Section, Segmented, Switch } from '../components/ui';
import { getModePref, getVoicePrefs, setVoicePrefs, type VoicePrefs } from '../app/prefs';
import { getCapabilities } from '../lib/translate';
import { playbackController } from '../lib/textToSpeech';
import { unlockAudio } from '../lib/audio';
import { VOICEMIRROR_FLAG } from '../engine/defaultDeps';

const SAMPLE: Record<string, string> = {
  en: 'This is how translations will sound.',
  es: 'Así sonarán las traducciones.',
  fr: 'Voici comment les traductions sonneront.',
  de: 'So werden Übersetzungen klingen.',
  it: 'Ecco come suoneranno le traduzioni.',
  pt: 'É assim que as traduções vão soar.',
  hu: 'Így fognak hangzani a fordítások.',
  nl: 'Zo zullen vertalingen klinken.',
  pl: 'Tak będą brzmieć tłumaczenia.',
  ru: 'Так будут звучать переводы.',
  uk: 'Так звучатимуть переклади.',
  tr: 'Çeviriler böyle duyulacak.',
  ar: 'هكذا ستبدو الترجمات عند سماعها.',
  fa: 'ترجمه‌ها این‌گونه شنیده می‌شوند.',
  he: 'כך יישמעו התרגומים.',
  hi: 'अनुवाद ऐसे सुनाई देंगे।',
  te: 'అనువాదాలు ఇలా వినిపిస్తాయి.',
  ta: 'மொழிபெயர்ப்புகள் இப்படித்தான் ஒலிக்கும்.',
  ja: '翻訳はこのように聞こえます。',
  ko: '번역은 이렇게 들립니다.',
  zh: '翻译听起来会是这样的。',
  id: 'Beginilah terjemahan akan terdengar.',
  vi: 'Bản dịch sẽ nghe như thế này.',
  th: 'คำแปลจะฟังดูแบบนี้',
  sv: 'Så här kommer översättningarna att låta.',
  da: 'Sådan kommer oversættelserne til at lyde.',
  no: 'Slik vil oversettelsene høres ut.',
  fi: 'Tältä käännökset kuulostavat.',
  cs: 'Takto budou překlady znít.',
  el: 'Έτσι θα ακούγονται οι μεταφράσεις.',
  ro: 'Așa vor suna traducerile.',
};
const VOLUMES = { quiet: 0.4, medium: 0.7, full: 1 } as const;
type VolumeKey = keyof typeof VOLUMES;
const volumeKey = (v: number): VolumeKey => (v <= 0.5 ? 'quiet' : v < 0.9 ? 'medium' : 'full');

interface DeviceVoice {
  name: string;
  local: boolean;
}

function useDeviceVoices(lang: string): DeviceVoice[] | null {
  const [voices, setVoices] = useState<DeviceVoice[] | null>(null);
  useEffect(() => {
    const synth = typeof window !== 'undefined' && 'speechSynthesis' in window ? window.speechSynthesis : null;
    if (!synth) return setVoices([]);
    const base = lang.split('-')[0].toLowerCase();
    const read = () =>
      setVoices(
        synth
          .getVoices()
          .filter((v) => v.lang.toLowerCase().startsWith(base))
          .map((v) => ({ name: v.name, local: v.localService }))
      );
    read();
    synth.addEventListener?.('voiceschanged', read);
    return () => synth.removeEventListener?.('voiceschanged', read);
  }, [lang]);
  return voices;
}

/** Voice Studio: how translations are spoken — preferences, voices on this device, and a real sample. */
export function VoicePage({ language }: { language: Language }) {
  const [prefs, setPrefs] = useState<VoicePrefs>(() => getVoicePrefs());
  const [serverTts, setServerTts] = useState<boolean | null>(null);
  const [sample, setSample] = useState<'idle' | 'playing' | 'failed'>('idle');
  const [sampleFailure, setSampleFailure] = useState<string | undefined>(undefined);
  const sampleId = useRef<string | null>(null);
  const voices = useDeviceVoices(language.bcp47);
  const onDevice = getModePref() === 'local-only';

  useEffect(() => {
    getCapabilities()
      .then((c) => setServerTts(c.tts))
      .catch(() => setServerTts(false));
    const pc = playbackController();
    const off = pc.onStatus((id, status, _engine, failure) => {
      if (id !== sampleId.current) return;
      if (status === 'started') setSample('playing');
      else if (status === 'failed') {
        setSampleFailure(failure);
        setSample('failed');
      }
      else if (status === 'ended' || status === 'cancelled' || status === 'dropped') setSample('idle');
    });
    return () => {
      off();
      if (sampleId.current) pc.cancelAll();
    };
  }, []);

  const update = (p: VoicePrefs) => {
    setPrefs(p);
    setVoicePrefs(p);
  };

  const playSample = () => {
    const pc = playbackController();
    if (sample === 'playing') {
      pc.cancelAll();
      return;
    }
    void unlockAudio();
    const id = `voice-sample-${Date.now()}`;
    sampleId.current = id;
    setSample('playing');
    pc.enqueue({
      segmentId: id,
      text: SAMPLE[language.code] ?? SAMPLE.en,
      lang: SAMPLE[language.code] ? language.bcp47 : 'en-US',
      volume: prefs.volume,
      ttsRoute: () => (onDevice ? 'device-only' : 'cloud-first'),
    });
  };

  const naturalVoice = onDevice ? 'Not used in On-device mode' : serverTts === null ? 'Checking…' : serverTts ? 'Available' : 'Not set up on this server';

  return (
    <Page eyebrow="Sound" title="Voice Studio" description="How translations are spoken to you.">
      <Section title="Spoken translations">
        <Switch label="Speak translations in AIR" description="Off: AIR shows translations on screen only." checked={prefs.speak} onChange={(v) => update({ ...prefs, speak: v })} />
        <div className="py-3">
          <Segmented<VolumeKey>
            label="Volume"
            value={volumeKey(prefs.volume)}
            onChange={(k) => update({ ...prefs, volume: VOLUMES[k] })}
            options={[
              { value: 'quiet', label: 'Quiet' },
              { value: 'medium', label: 'Medium' },
              { value: 'full', label: 'Full' },
            ]}
          />
          <p className="mt-2 text-label text-fg-muted">Keep it low enough to still hear the people around you.</p>
        </div>
        <div className="flex min-h-14 items-center justify-between gap-4 py-2.5">
          <div>
            <p className="text-body-lg font-[520]">Sample in {language.native}</p>
            <p className="text-label text-fg-muted" aria-live="polite">
              {sample === 'playing' ? 'Playing…' : sample === 'failed' ? (sampleFailure === 'blocked' ? 'The browser blocked sound. Tap Play sample again.' : sampleFailure === 'no-voice' ? `No ${language.name} voice on this device.` : 'No voice could play this. Check your device’s sound and voices.') : 'Uses the same voice route as translations.'}
            </p>
          </div>
          <Button size="sm" variant="tertiary" onClick={playSample} icon={sample === 'playing' ? <Square className="h-4 w-4" /> : <Play className="h-4 w-4" />}>
            {sample === 'playing' ? 'Stop' : 'Play'}
          </Button>
        </div>
      </Section>

      <Section title="Voices">
        <div className="flex min-h-12 items-center justify-between gap-4 py-2.5">
          <span className="text-body-lg font-[520]">Natural voice (server)</span>
          <span className="text-label text-fg-muted">{naturalVoice}</span>
        </div>
        <div className="py-3">
          <p className="text-body-lg font-[520]">On this device, for {language.native}</p>
          {voices === null ? (
            <p className="mt-1 text-label text-fg-muted">Checking…</p>
          ) : voices.length === 0 ? (
            <p className="mt-1 text-label text-fg-muted">No device voice for this language. Translations are spoken by the natural voice when available, otherwise shown only.</p>
          ) : (
            <ul className="mt-2 space-y-1 text-label text-fg-muted" aria-label="Device voices">
              {voices.slice(0, 8).map((v) => (
                <li key={v.name} className="flex justify-between gap-4">
                  <span className="truncate text-fg">{v.name}</span>
                  <span className="shrink-0">{v.local ? 'On device' : 'Network'}</span>
                </li>
              ))}
              {voices.length > 8 && <li>and {voices.length - 8} more</li>}
            </ul>
          )}
          <p className="mt-2 text-label text-fg-muted">Talktual picks a matching voice automatically.</p>
        </div>
      </Section>

      <Section title="Voice match">
        <p className="py-3 text-body text-fg-muted">
          {VOICEMIRROR_FLAG
            ? 'In calls, you can let the other person hear your translations in a voice closer to yours. It adjusts pitch and pace only — it does not copy or clone your voice — and it is off until you allow it in each call.'
            : 'Not included in this version. Translations use a standard voice.'}
        </p>
      </Section>
    </Page>
  );
}
