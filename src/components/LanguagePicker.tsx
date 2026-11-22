import { useEffect, useMemo, useState } from 'react';
import { Check, Search } from 'lucide-react';
import { Language, SUPPORTED_LANGUAGES } from '../types';
import { Sheet } from './ui';
import { cn } from '../utils';

interface LanguagePickerProps {
  open: boolean;
  title?: string;
  selected: Language;
  onSelect: (language: Language) => void;
  onClose: () => void;
}

export function LanguagePicker({ open, title = 'Your language', selected, onSelect, onClose }: LanguagePickerProps) {
  const [query, setQuery] = useState('');
  useEffect(() => {
    if (open) setQuery('');
  }, [open]);

  const results = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return SUPPORTED_LANGUAGES;
    return SUPPORTED_LANGUAGES.filter((l) => l.name.toLowerCase().includes(q) || l.native.toLowerCase().includes(q) || l.code === q);
  }, [query]);

  return (
    <Sheet open={open} onClose={onClose} title={title} tall>
      <div className="sticky top-0 z-10 bg-raised pb-2">
        <label className="flex h-11 items-center gap-2 rounded-md bg-surface px-3">
          <Search className="h-4 w-4 shrink-0 text-fg-subtle" aria-hidden="true" />
          <input
            data-autofocus
            aria-label="Search languages"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search"
            className="w-full bg-transparent text-body text-fg outline-none placeholder:text-fg-subtle"
          />
        </label>
      </div>
      {results.length === 0 && <p className="py-10 text-center text-body text-fg-muted">No language matches “{query}”.</p>}
      <ul className="divide-y divide-line">
        {results.map((language) => {
          const isSelected = language.code === selected.code;
          return (
            <li key={language.code}>
              <button
                type="button"
                aria-pressed={isSelected}
                onClick={() => {
                  onSelect(language);
                  onClose();
                }}
                className={cn('flex min-h-12 w-full items-center gap-3 py-2 text-left', isSelected && 'font-semibold')}
              >
                <span className="text-[1.25rem] leading-none" aria-hidden="true">
                  {language.flag}
                </span>
                <span className="min-w-0 flex-1 truncate text-body text-fg">
                  {language.native}
                  {language.native !== language.name && <span className="font-normal text-fg-muted"> · {language.name}</span>}
                </span>
                {isSelected && <Check className="h-5 w-5 shrink-0 text-accent-text" aria-hidden="true" />}
              </button>
            </li>
          );
        })}
      </ul>
    </Sheet>
  );
}
