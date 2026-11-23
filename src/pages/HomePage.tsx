import { useState } from 'react';
import { ArrowRight, ChevronRight, Settings } from 'lucide-react';
import type { Language } from '../types';
import type { Route } from '../app/routes';
import { getModePref, getVoicePrefs, MODE_LABEL } from '../app/prefs';
import { Wordmark } from '../components/Brand';
import { Button, IconButton } from '../components/ui';
import { LanguagePicker } from '../components/LanguagePicker';

interface HomeProps {
  language: Language;
  airFrom: Language;
  onLanguageChange: (l: Language) => void;
  onAirFromChange: (l: Language) => void;
  onStartAir: () => void;
  go: (r: Route) => void;
}

const MORE: { route: Route; title: string; detail: string }[] = [
  { route: 'connect', title: 'Connect', detail: 'A translated call with one other person' },
  { route: 'voice', title: 'Voice Studio', detail: 'How translations sound' },
  { route: 'lens', title: 'Lens', detail: 'Camera translation — not available in this version' },
  { route: 'rewind', title: 'Rewind', detail: 'Talktual keeps nothing after you leave' },
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
  return (
    <main className="mx-auto w-full max-w-[720px] px-4 pb-10 sm:px-6 lg:px-10">
      <header className="safe-t flex h-16 items-center justify-between lg:hidden">
        <Wordmark />
        <IconButton label="Settings" onClick={() => go('settings')}>
          <Settings className="h-5 w-5" />
        </IconButton>
      </header>

      <section aria-labelledby="air-title" className="pt-6 lg:pt-16">
        <p className="text-label font-semibold text-accent-text">AIR</p>
        <h1 id="air-title" className="mt-1 text-title font-semibold tracking-[-0.01em]">
          Translate the room around you
        </h1>
        <p className="mt-2 max-w-[30rem] text-body text-fg-muted">Hear nearby speech in your language, as it happens. It listens only while you want it to.</p>

        <div className="mt-6 grid grid-cols-[1fr_auto_1fr] items-stretch border-y border-line" role="group" aria-label="Language pair">
          <button type="button" onClick={() => setPicker('from')} className="min-w-0 py-3 pr-3 text-left" aria-label={`They speak: ${airFrom.native}. Change`}>
            <span className="block text-label text-fg-muted">They speak</span>
            <span className="block truncate text-heading font-semibold">{airFrom.native}</span>
          </button>
          <span className="flex items-center px-3 text-fg-subtle" aria-hidden="true">
            <ArrowRight className="h-4 w-4" />
          </span>
          <button type="button" onClick={() => setPicker('to')} className="min-w-0 py-3 pl-3 text-right" aria-label={`You hear: ${language.native}. Change`}>
            <span className="block text-label text-fg-muted">You hear</span>
            <span className="block truncate text-heading font-semibold">{language.native}</span>
          </button>
        </div>
        <p className="mt-3 text-label text-fg-muted" data-testid="home-air-status">
          {[MODE_LABEL[mode] + ' processing', voice.speak ? 'Spoken and on screen' : 'On screen only'].join(' · ')}
        </p>
        <Button variant="primary" className="mt-6 w-full sm:w-auto sm:min-w-[220px]" onClick={onStartAir}>
          Start listening
        </Button>
      </section>

      <section aria-label="More" className="mt-12">
        <ul className="divide-y divide-line border-y border-line">
          {MORE.map((m) => (
            <li key={m.route}>
              <a
                href={`#/${m.route}`}
                onClick={(e) => {
                  e.preventDefault();
                  go(m.route);
                }}
                className="flex min-h-[60px] items-center gap-3 py-3 transition-colors hover:text-fg"
              >
                <span className="min-w-0 flex-1">
                  <span className="block text-body font-semibold">{m.title}</span>
                  <span className="block text-label text-fg-muted">{m.detail}</span>
                </span>
                <ChevronRight className="h-5 w-5 shrink-0 text-fg-subtle" aria-hidden="true" />
              </a>
            </li>
          ))}
        </ul>
      </section>

      <p className="mt-10 text-meta text-fg-subtle">
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
