import { useEffect, useMemo, useRef, useState } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { Check, Search, X } from 'lucide-react';
import { Language, SUPPORTED_LANGUAGES } from '../types';
import { cn } from '../utils';

interface LanguagePickerProps {
  open: boolean;
  title?: string;
  subtitle?: string;
  selected: Language;
  onSelect: (language: Language) => void;
  onClose: () => void;
}

export function LanguagePicker({
  open,
  title = 'Your language',
  subtitle,
  selected,
  onSelect,
  onClose,
}: LanguagePickerProps) {
  const [query, setQuery] = useState('');
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (open) setQuery('');
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  const results = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return SUPPORTED_LANGUAGES;
    return SUPPORTED_LANGUAGES.filter(
      (l) =>
        l.name.toLowerCase().includes(q) ||
        l.native.toLowerCase().includes(q) ||
        l.code.includes(q)
    );
  }, [query]);

  return (
    <AnimatePresence>
      {open && (
        <>
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
            onClick={onClose}
            className="fixed inset-0 z-40 bg-black/60 backdrop-blur-[2px]"
          />

          <motion.div
            initial={{ y: '100%' }}
            animate={{ y: 0 }}
            exit={{ y: '100%' }}
            transition={{ type: 'spring', damping: 32, stiffness: 340 }}
            className="fixed inset-x-0 bottom-0 z-50 mx-auto flex max-h-[82vh] w-full max-w-md flex-col overflow-hidden rounded-t-[28px] border border-white/10 bg-[#141418]/95 backdrop-blur-2xl"
            style={{ boxShadow: '0 -20px 60px rgba(0,0,0,0.6)' }}
          >
            <div className="flex justify-center pt-3 pb-1">
              <div className="h-1 w-9 rounded-full bg-white/20" />
            </div>

            <div className="flex items-start justify-between px-5 pt-2 pb-3">
              <div>
                <h2 className="text-[19px] font-semibold tracking-[-0.01em] text-[var(--text-1)]">
                  {title}
                </h2>
                {subtitle && (
                  <p className="mt-0.5 text-[13px] text-[var(--text-2)]">{subtitle}</p>
                )}
              </div>
              <button
                onClick={onClose}
                aria-label="Close"
                className="-mr-1 -mt-1 flex h-8 w-8 items-center justify-center rounded-full bg-white/8 text-[var(--text-2)] transition-colors hover:bg-white/14"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="px-5 pb-3">
              <div className="flex items-center gap-2 rounded-xl bg-white/8 px-3 py-2.5">
                <Search className="h-4 w-4 shrink-0 text-[var(--text-3)]" />
                <input
                  autoFocus
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder="Search languages"
                  className="w-full bg-transparent text-[15px] text-[var(--text-1)] outline-none placeholder:text-[var(--text-3)]"
                />
              </div>
            </div>

            <div ref={listRef} className="sheet-scroll flex-1 overflow-y-auto px-3 pb-2">
              {results.length === 0 && (
                <p className="px-3 py-10 text-center text-[15px] text-[var(--text-3)]">
                  No languages match “{query}”
                </p>
              )}

              {results.map((language) => {
                const isSelected = language.code === selected.code;
                return (
                  <button
                    key={language.code}
                    onClick={() => {
                      onSelect(language);
                      onClose();
                    }}
                    className={cn(
                      'flex w-full items-center gap-3 rounded-2xl px-3 py-3 text-left transition-colors',
                      isSelected ? 'bg-white/10' : 'hover:bg-white/6 active:bg-white/10'
                    )}
                  >
                    <span className="text-[22px] leading-none">{language.flag}</span>
                    {/* Fixed row height keeps the list even, whether or not a
                        language needs its English name spelled out too. */}
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
                    {isSelected && <Check className="h-[18px] w-[18px] shrink-0 text-[var(--blue)]" />}
                  </button>
                );
              })}
            </div>

            <div className="safe-b px-5 pt-2" />
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
