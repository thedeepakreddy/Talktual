import type { Ref } from 'react';
import { ChevronRight } from 'lucide-react';
import type { Language } from '../types';

/** "Your language" field: shows the choice, opens the picker. */
export function LanguageField({ label = 'Your language', language, onOpen }: { label?: string; language: Language; onOpen: () => void }) {
  return (
    <div>
      <span className="mb-1.5 block text-label font-medium text-fg-muted">{label}</span>
      <button
        type="button"
        onClick={onOpen}
        aria-label={`${label}: ${language.native}. Change`}
        className="flex h-12 w-full items-center gap-3 rounded-md border border-line bg-surface px-3.5 text-left transition-colors hover:bg-surface-hover"
      >
        <span className="text-[1.25rem] leading-none" aria-hidden="true">
          {language.flag}
        </span>
        <span className="min-w-0 flex-1 truncate text-body text-fg">
          {language.native}
          {language.native !== language.name && <span className="text-fg-muted"> · {language.name}</span>}
        </span>
        <ChevronRight className="h-4 w-4 shrink-0 text-fg-subtle" aria-hidden="true" />
      </button>
    </div>
  );
}

/** Text field with a visible label and an inline error. */
export function NameField({ value, onChange, error, inputRef }: { value: string; onChange: (v: string) => void; error?: string | null; inputRef?: Ref<HTMLInputElement> }) {
  return (
    <div>
      <label htmlFor="name" className="mb-1.5 block text-label font-medium text-fg-muted">
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
        className="h-12 w-full rounded-md border border-line bg-surface px-3.5 text-body text-fg outline-none transition-colors placeholder:text-fg-subtle focus:border-focus aria-[invalid=true]:border-danger"
      />
      {error && (
        <p id="name-error" className="mt-1.5 text-label text-danger">
          {error}
        </p>
      )}
    </div>
  );
}
