import type { Ref } from 'react';
import { ChevronRight } from './icons';
import type { Language } from '../types';

/** "Your language" field: shows the choice, opens the picker. */
export function LanguageField({ label = 'Your language', language, onOpen }: { label?: string; language: Language; onOpen: () => void }) {
  return (
    <div>
      <span className="t-eyebrow mb-2 block px-1 text-fg-subtle">{label}</span>
      <button
        type="button"
        onClick={onOpen}
        aria-label={`${label}: ${language.native}. Change`}
        className="glass-clear press flex h-14 w-full items-center gap-3 rounded-full pl-4 pr-4 text-left hover:brightness-110"
      >
        <span className="text-[1.25rem] leading-none" aria-hidden="true">
          {language.flag}
        </span>
        <span className="min-w-0 flex-1 truncate text-body-lg font-[560] text-fg">
          {language.native}
          {language.native !== language.name && <span className="text-fg-muted"> · {language.name}</span>}
        </span>
        <ChevronRight className="h-[18px] w-[18px] shrink-0 text-fg-subtle" aria-hidden="true" />
      </button>
    </div>
  );
}

/** Text field with a visible label and an inline error. */
export function NameField({ value, onChange, error, inputRef }: { value: string; onChange: (v: string) => void; error?: string | null; inputRef?: Ref<HTMLInputElement> }) {
  return (
    <div>
      <label htmlFor="name" className="t-eyebrow mb-2 block px-1 text-fg-subtle">
        Your name
      </label>
      <input
        ref={inputRef}
        id="name"
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        autoComplete="given-name"
        maxLength={40}
        aria-invalid={Boolean(error)}
        aria-describedby={error ? 'name-error' : undefined}
        placeholder="How they’ll see you"
        className="glass-clear h-14 w-full rounded-full px-5 text-body-lg font-[520] text-fg outline-none transition-shadow placeholder:font-normal placeholder:text-fg-subtle focus:shadow-[inset_0_0_0_1.5px_var(--focus),0_0_0_4px_color-mix(in_oklab,var(--accent)_16%,transparent)] aria-[invalid=true]:shadow-[inset_0_0_0_1.5px_var(--danger)]"
      />
      {error && (
        <p id="name-error" className="mt-2 px-1 text-label text-danger">
          {error}
        </p>
      )}
    </div>
  );
}
