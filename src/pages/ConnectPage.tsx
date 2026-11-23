import { useRef, useState } from 'react';
import type { Language } from '../types';
import { Page } from '../components/shell/AppShell';
import { LanguageField, NameField } from '../components/LanguageField';
import { LanguagePicker } from '../components/LanguagePicker';
import { Button } from '../components/ui';

interface ConnectProps {
  language: Language;
  onLanguageChange: (l: Language) => void;
  userName: string;
  onUserNameChange: (n: string) => void;
  onStart: () => void;
  onJoin: () => void;
}

/** Connect entry: start or join a translated call. Name + language are the only setup. */
export function ConnectPage({ language, onLanguageChange, userName, onUserNameChange, onStart, onJoin }: ConnectProps) {
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
    <Page title="Connect" description="A translated call with one other person. You speak yours, they hear theirs — and back again.">
      <form
        className="mt-6 max-w-[420px] space-y-4"
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
        <LanguageField label="You speak" language={language} onOpen={() => setPickerOpen(true)} />
        <div className="flex flex-col gap-3 pt-3 sm:flex-row">
          <Button type="submit" variant="primary" className="sm:min-w-[180px]">
            Start a call
          </Button>
          <Button variant="secondary" onClick={() => go(onJoin)}>
            Join with a code
          </Button>
        </div>
      </form>
      <p className="mt-8 max-w-[420px] text-label text-fg-muted">The other person opens your link or scans your code. Talktual connects you directly when it can and doesn’t save the conversation.</p>
      <LanguagePicker open={pickerOpen} title="You speak" selected={language} onSelect={onLanguageChange} onClose={() => setPickerOpen(false)} />
    </Page>
  );
}
