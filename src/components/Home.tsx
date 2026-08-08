import { useState } from 'react';
import { motion } from 'motion/react';
import { ArrowRight, ScanLine } from 'lucide-react';
import { SUPPORTED_LANGUAGES, Language, detectLanguage } from '../types';
import { LanguagePicker } from './LanguagePicker';

interface HomeProps {
  onStartCall: (lang: Language, name: string) => void;
  onJoinCall: (lang: Language, name: string) => void;
}

export function Home({ onStartCall, onJoinCall }: HomeProps) {
  const [language, setLanguage] = useState<Language>(() => detectLanguage());
  const [userName, setUserName] = useState('');
  const [pickerOpen, setPickerOpen] = useState(false);

  const ready = userName.trim().length > 0;

  return (
    <div className="mx-auto flex min-h-screen w-full max-w-md flex-col px-6">
      <motion.div
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.5, ease: [0.32, 0.72, 0, 1] }}
        className="flex flex-1 flex-col"
      >
        <div className="safe-t flex flex-1 flex-col items-center justify-center py-10">
          <AppMark />

          <h1 className="mt-6 text-[34px] font-bold leading-none tracking-[-0.03em] text-[var(--text-1)]">
            Talktual
          </h1>
          <p className="mt-3 max-w-[17rem] text-center text-[15px] leading-snug text-[var(--text-2)]">
            Hold to talk. They hear you in their language, you hear them in yours.
          </p>
        </div>

        <div className="space-y-3 pb-6">
          <div>
            <label
              htmlFor="name"
              className="mb-2 block px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-[var(--text-3)]"
            >
              Your name
            </label>
            <input
              id="name"
              type="text"
              value={userName}
              onChange={(e) => setUserName(e.target.value)}
              placeholder="So they know who’s talking"
              autoComplete="given-name"
              className="w-full rounded-2xl border border-white/8 bg-white/[0.06] px-4 py-3.5 text-[16px] text-[var(--text-1)] outline-none transition-colors placeholder:text-[var(--text-3)] focus:border-[var(--blue)]/60 focus:bg-white/[0.09]"
            />
          </div>

          <div>
            <span className="mb-2 block px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-[var(--text-3)]">
              You speak
            </span>
            <button
              onClick={() => setPickerOpen(true)}
              className="flex w-full items-center gap-3 rounded-2xl border border-white/8 bg-white/[0.06] px-4 py-3.5 text-left transition-colors hover:bg-white/[0.09] active:bg-white/[0.12]"
            >
              <span className="text-[24px] leading-none">{language.flag}</span>
              <span className="flex min-h-[38px] min-w-0 flex-1 flex-col justify-center">
                <span className="block truncate text-[16px] font-medium text-[var(--text-1)]">
                  {language.native}
                </span>
                {language.native !== language.name && (
                  <span className="block truncate text-[13px] text-[var(--text-2)]">
                    {language.name}
                  </span>
                )}
              </span>
              <span className="text-[13px] font-medium text-[var(--blue)]">Change</span>
            </button>
            <p className="mt-2 px-1 text-[12px] leading-snug text-[var(--text-3)]">
              {SUPPORTED_LANGUAGES.length} languages. You can switch mid-call.
            </p>
          </div>
        </div>

        <div className="safe-b space-y-3">
          <button
            onClick={() => onStartCall(language, userName.trim())}
            disabled={!ready}
            className="flex w-full items-center justify-center gap-2 rounded-2xl bg-[var(--blue)] py-4 text-[16px] font-semibold text-white transition-all hover:brightness-110 active:scale-[0.985] disabled:pointer-events-none disabled:opacity-30"
          >
            Start a channel
            <ArrowRight className="h-[18px] w-[18px]" />
          </button>
          <button
            onClick={() => onJoinCall(language, userName.trim())}
            disabled={!ready}
            className="flex w-full items-center justify-center gap-2 rounded-2xl border border-white/12 bg-white/[0.05] py-4 text-[16px] font-semibold text-[var(--text-1)] transition-all hover:bg-white/[0.09] active:scale-[0.985] disabled:pointer-events-none disabled:opacity-30"
          >
            <ScanLine className="h-[18px] w-[18px]" />
            Join a channel
          </button>

          <p className="pt-1 text-center text-[12px] text-[var(--text-3)]">
            by{' '}
            <a
              href="https://askdeepakai-datascientist.onrender.com/"
              target="_blank"
              rel="noopener noreferrer"
              className="text-[var(--text-2)] underline-offset-2 transition-colors hover:text-[var(--text-1)] hover:underline"
            >
              AskDeepakAI
            </a>
          </p>
        </div>
      </motion.div>

      <LanguagePicker
        open={pickerOpen}
        selected={language}
        subtitle="What you'll speak into the mic"
        onSelect={setLanguage}
        onClose={() => setPickerOpen(false)}
      />
    </div>
  );
}

/** App mark: a machined dial with a transmit arc, echoing the PTT button. */
function AppMark() {
  return (
    <div
      className="relative flex h-[76px] w-[76px] items-center justify-center rounded-[22px]"
      style={{
        background: 'linear-gradient(160deg, #2b2b31 0%, #17171b 60%, #101014 100%)',
        boxShadow:
          '0 18px 36px -12px rgba(0,0,0,0.8), inset 0 1px 0 rgba(255,255,255,0.14), inset 0 -1px 0 rgba(0,0,0,0.5)',
      }}
    >
      <svg width="38" height="38" viewBox="0 0 38 38" fill="none" aria-hidden="true">
        <circle cx="19" cy="19" r="5.5" fill="var(--amber)" />
        <path
          d="M27.2 10.8a11.6 11.6 0 0 1 0 16.4M30.9 7.1a16.8 16.8 0 0 1 0 23.8"
          stroke="var(--amber)"
          strokeOpacity="0.75"
          strokeWidth="2.4"
          strokeLinecap="round"
        />
        <path
          d="M10.8 27.2a11.6 11.6 0 0 1 0-16.4M7.1 30.9a16.8 16.8 0 0 1 0-23.8"
          stroke="var(--text-3)"
          strokeWidth="2.4"
          strokeLinecap="round"
        />
      </svg>
    </div>
  );
}
