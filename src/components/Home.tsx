import { useRef, useState } from 'react';
import type { Language } from '../types';
import { LanguagePicker } from './LanguagePicker';
import { Wordmark } from './Brand';
import { LanguageField, NameField } from './LanguageField';
import { Ear, ChevronRight } from 'lucide-react';
import { Button } from './ui';

interface HomeProps {
  language: Language;
  onLanguageChange: (language: Language) => void;
  userName: string;
  onUserNameChange: (name: string) => void;
  onStart: () => void;
  onJoin: () => void;
  /** Phase 8: translate the room around you (no name or second person needed). */
  onAmbient?: () => void;
}

export function Home({ language, onLanguageChange, userName, onUserNameChange, onStart, onJoin, onAmbient }: HomeProps) {
  const [pickerOpen, setPickerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nameRef = useRef<HTMLInputElement>(null);

  const go = (next: () => void) => {
    if (!userName.trim()) {
      setError('Add your name so the other person knows who’s talking.');
      nameRef.current?.focus();
      return;
    }
    next();
  };

  return (
    <main className="mx-auto flex min-h-dvh w-full max-w-[420px] flex-col px-4 sm:px-6">
      <header className="safe-t flex h-14 items-center">
        <Wordmark />
      </header>

      <div className="flex flex-1 flex-col justify-center py-8">
        <h1 className="text-title font-semibold tracking-[-0.01em]">Talk across languages</h1>
        <p className="mt-2 text-body text-fg-muted">You speak yours, they hear theirs — and back again.</p>

        <form
          className="mt-8 space-y-4"
          onSubmit={(e) => {
            e.preventDefault();
            go(onStart);
          }}
        >
          <NameField
            inputRef={nameRef}
            value={userName}
            error={error}
            onChange={(v) => {
              onUserNameChange(v);
              if (v.trim()) setError(null);
            }}
          />
          <LanguageField language={language} onOpen={() => setPickerOpen(true)} />
          <div className="space-y-3 pt-4">
            <Button type="submit" variant="primary" block>
              Start a call
            </Button>
            <Button variant="secondary" block onClick={() => go(onJoin)}>
              Join with a code
            </Button>
          </div>
        </form>
        {onAmbient && (
          <div className="mt-8 border-t border-line pt-6">
            <button type="button" onClick={onAmbient} className="flex min-h-14 w-full items-center gap-3 rounded-md text-left hover:bg-surface">
              <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-surface text-fg" aria-hidden="true">
                <Ear className="h-5 w-5" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block text-body font-semibold">Translate what’s around you</span>
                <span className="block text-label text-fg-muted">Hear nearby conversation in {language.name}. No one else needs the app.</span>
              </span>
              <ChevronRight className="h-5 w-5 shrink-0 text-fg-subtle" aria-hidden="true" />
            </button>
          </div>
        )}
      </div>

      <footer className="safe-b pt-4 text-center text-meta text-fg-subtle">
        by{' '}
        <a href="https://askdeepakai-datascientist.onrender.com/" target="_blank" rel="noopener noreferrer" className="inline-flex min-h-11 items-center text-fg-muted underline-offset-2 hover:underline">
          AskDeepakAI
        </a>
      </footer>

      <LanguagePicker open={pickerOpen} selected={language} onSelect={onLanguageChange} onClose={() => setPickerOpen(false)} />
    </main>
  );
}
