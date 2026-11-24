import { useEffect, useState } from 'react';
import { ArrowRight, ArrowUpRight, AudioLines, History, PhoneCall, ScanText, Settings } from '../components/icons';
import type { Language } from '../types';
import type { Route } from '../app/routes';
import { getModePref, getVoicePrefs, MODE_LABEL } from '../app/prefs';
import { Wordmark } from '../components/Brand';
import { Button, IconButton } from '../components/ui';
import { LanguagePicker } from '../components/LanguagePicker';
import { micPermission } from '../lib/audio';
import { detectBrowserCapabilities } from '../engine/capabilities';
import { LogoWave } from '../three/LogoWave';

interface HomeProps {
  language: Language;
  airFrom: Language;
  onLanguageChange: (l: Language) => void;
  onAirFromChange: (l: Language) => void;
  onStartAir: () => void;
  go: (r: Route) => void;
}

const MORE: { route: Route; title: string; detail: string; icon: typeof PhoneCall }[] = [
  { route: 'connect', title: 'Connect', detail: 'A translated call with one other person', icon: PhoneCall },
  { route: 'voice', title: 'Voice Studio', detail: 'How translations sound', icon: AudioLines },
  { route: 'lens', title: 'Lens', detail: 'Camera translation — not available in this version', icon: ScanText },
  { route: 'rewind', title: 'Rewind', detail: 'Talktual keeps nothing after you leave', icon: History },
];

/**
 * Home: AIR is the primary action, composed into the page (not a hero card);
 * other destinations are a quiet list. State shown is real: saved processing
 * mode and speech preference — no fabricated device or activity status.
 */
export function HomePage({ language, airFrom, onLanguageChange, onAirFromChange, onStartAir, go }: HomeProps) {
  const [picker, setPicker] = useState<'from' | 'to' | null>(null);
  const mode = getModePref();
  const voice = getVoicePrefs();
  // Real blockers, stated before Start (not discovered after it).
  const [blocker, setBlocker] = useState<string | null>(null);
  useEffect(() => {
    const caps = typeof window !== 'undefined' ? detectBrowserCapabilities(window) : null;
    if (caps && !caps.secureContext) return setBlocker('Listening needs a secure (https) connection.');
    if (caps && !caps.webSpeech) return setBlocker('This browser can’t recognise speech. Use Chrome, Edge or Safari for AIR.');
    let live = true;
    micPermission()
      .then((p) => live && p === 'denied' && setBlocker('Microphone access is blocked for this site. Allow it in your browser’s site settings.'))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);
  return (
    <main className="mx-auto w-full max-w-[1120px] px-4 pb-12 sm:px-6 lg:px-12">
      <header className="safe-t flex h-[68px] items-center justify-between lg:hidden">
        <Wordmark />
        <IconButton label="Settings" onClick={() => go('settings')}>
          <Settings className="h-[19px] w-[19px]" />
        </IconButton>
      </header>

      <div className="lg:grid lg:grid-cols-[minmax(0,1fr)_minmax(360px,520px)] lg:items-center lg:gap-14 lg:pt-14">
        <div className="relative -mx-4 mt-1 h-[236px] sm:-mx-6 sm:h-[280px] lg:order-2 lg:mx-0 lg:mt-0 lg:h-[460px]">
          <LogoWave ambient className="absolute inset-0" />
        </div>

        <section aria-labelledby="air-title" className="relative pt-6 lg:order-1 lg:pt-0">
          <p className="t-eyebrow text-accent-text">AIR · Live room translation</p>
          <h1 id="air-title" className="t-display mt-3 text-display lg:text-display-lg">
            Translate the room <span className="t-accent text-ember pr-1">around you</span>
          </h1>
          <p className="mt-4 max-w-[30rem] text-body-lg text-fg-muted">Hear nearby speech in your language, as it happens. It listens only while you want it to.</p>

          <div className="glass-strong mt-7 rounded-[32px] p-2">
            <div className="grid grid-cols-[1fr_auto_1fr] items-stretch gap-1.5" role="group" aria-label="Language pair">
              <button type="button" onClick={() => setPicker('from')} className="glass-clear press min-w-0 rounded-[24px] px-4 py-3.5 text-left" aria-label={`They speak: ${airFrom.native}. Change`}>
                <span className="t-eyebrow block text-fg-subtle">They speak</span>
                <span className="mt-1 block truncate text-heading font-[640] tracking-[-0.015em]">{airFrom.native}</span>
              </button>
              <span className="flex items-center" aria-hidden="true">
                <span className="orb-core flex h-9 w-9 items-center justify-center rounded-full text-on-accent">
                  <ArrowRight className="h-4 w-4" strokeWidth={2.2} />
                </span>
              </span>
              <button type="button" onClick={() => setPicker('to')} className="glass-clear press min-w-0 rounded-[24px] px-4 py-3.5 text-right" aria-label={`You hear: ${language.native}. Change`}>
                <span className="t-eyebrow block text-fg-subtle">You hear</span>
                <span className="mt-1 block truncate text-heading font-[640] tracking-[-0.015em]">{language.native}</span>
              </button>
            </div>
            <div className="px-3 pb-1 pt-3">
              <p className="flex items-center gap-2 text-label text-fg-muted" data-testid="home-air-status">
                <span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-[var(--fg-subtle)]" />
                {[MODE_LABEL[mode] + ' processing', voice.speak ? 'Spoken and on screen' : 'On screen only'].join(' · ')}
              </p>
              {blocker && (
                <p role="status" className="mt-2 text-label font-medium text-warning">
                  {blocker}
                </p>
              )}
            </div>
            <Button variant="primary" size="lg" block className="mt-2 justify-between pl-7 pr-[22px]" chip={<ArrowRight />} onClick={onStartAir}>
              Start listening
            </Button>
          </div>
        </section>
      </div>

      <section aria-labelledby="more-title" className="mt-12 lg:mt-20">
        <h2 id="more-title" className="t-eyebrow px-1 text-fg-subtle">
          More ways to talk
        </h2>
        <ul className="no-scrollbar fade-x -mx-4 mt-3 flex snap-x snap-mandatory gap-3 overflow-x-auto px-4 pb-6 pt-1 sm:mx-0 sm:grid sm:grid-cols-2 sm:overflow-visible sm:px-0 sm:[mask-image:none] lg:grid-cols-4">
          {MORE.map((m) => {
            const Icon = m.icon;
            return (
              <li key={m.route} className="w-[min(72vw,236px)] shrink-0 snap-start sm:w-auto">
                <a
                  href={`#/${m.route}`}
                  onClick={(e) => {
                    e.preventDefault();
                    go(m.route);
                  }}
                  className="glass press group flex h-full min-h-[164px] flex-col rounded-[28px] p-4"
                >
                  <span className="flex items-start justify-between">
                    <span className="flex h-11 w-11 items-center justify-center rounded-[15px] text-accent-text" style={{ background: 'linear-gradient(160deg, color-mix(in oklab, var(--accent) 28%, transparent), color-mix(in oklab, var(--accent) 8%, transparent))', boxShadow: 'inset 0 1px 0 var(--glass-hi), inset 0 0 0 1px color-mix(in oklab, var(--accent) 30%, transparent)' }}>
                      <Icon className="h-5 w-5" aria-hidden="true" />
                    </span>
                    <ArrowUpRight className="h-[18px] w-[18px] text-fg-subtle transition-transform duration-300 group-hover:-translate-y-0.5 group-hover:translate-x-0.5" aria-hidden="true" />
                  </span>
                  <span className="mt-auto block pt-6 text-body-lg font-[640] tracking-[-0.015em]">{m.title}</span>
                  <span className="mt-1 block min-h-[2.25rem] text-label text-fg-muted">{m.detail}</span>
                </a>
              </li>
            );
          })}
        </ul>
      </section>

      <p className="mt-6 text-meta text-fg-subtle">
        Talktual by{' '}
        <a href="https://askdeepakai-datascientist.onrender.com/" target="_blank" rel="noopener noreferrer" className="inline-flex min-h-11 items-center underline-offset-2 hover:underline">
          AskDeepakAI
        </a>
      </p>

      <LanguagePicker
        open={picker !== null}
        title={picker === 'from' ? 'They speak' : 'You hear'}
        selected={picker === 'from' ? airFrom : language}
        onSelect={(l) => (picker === 'from' ? onAirFromChange(l) : onLanguageChange(l))}
        onClose={() => setPicker(null)}
      />
    </main>
  );
}
