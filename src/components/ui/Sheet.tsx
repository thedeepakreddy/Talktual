import { useEffect, useId, useRef, type ReactNode } from 'react';
import { AnimatePresence, motion } from 'motion/react';
import { X, ChevronLeft } from '../icons';
import { IconButton } from './IconButton';
import { cn } from '../../utils';

export interface SheetProps {
  open: boolean;
  onClose: () => void;
  title: string;
  /** Optional back action (sub-page inside a sheet). */
  onBack?: () => void;
  /** Accessible name of the close button (default "Close"). */
  closeLabel?: string;
  children: ReactNode;
  footer?: ReactNode;
  /** Hide the close button (e.g. a consent decision must be explicit). */
  dismissible?: boolean;
  /** Fill most of the height on phones (lists). */
  tall?: boolean;
}

/**
 * Dialog surface. Bottom sheet on phones, right-side panel on wide screens
 * (so the conversation stays visible). Escape closes; focus moves into the
 * sheet on open and returns to the opener on close.
 */
export function Sheet({ open, onClose, title, onBack, closeLabel = 'Close', children, footer, dismissible = true, tall }: SheetProps) {
  const titleId = useId();
  const panelRef = useRef<HTMLDivElement>(null);
  const returnTo = useRef<HTMLElement | null>(null);

  // Callers pass inline handlers that change on every engine update; keep the
  // latest in refs so the open/close effect runs only when `open` changes
  // (re-running it would bounce focus back to the opener mid-call).
  const onCloseRef = useRef(onClose);
  const dismissibleRef = useRef(dismissible);
  onCloseRef.current = onClose;
  dismissibleRef.current = dismissible;

  useEffect(() => {
    if (!open) return;
    returnTo.current = document.activeElement as HTMLElement | null;
    const focusables = () =>
      panelRef.current ? [...panelRef.current.querySelectorAll<HTMLElement>('button:not([disabled]),input:not([disabled]),[href],[tabindex]:not([tabindex="-1"])')] : [];
    const t = setTimeout(() => {
      const panel = panelRef.current;
      if (!panel) return;
      const first = panel.querySelector<HTMLElement>('[data-autofocus]') ?? panel.querySelector<HTMLElement>('h2');
      first?.focus();
    }, 30);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && dismissibleRef.current) {
        e.stopPropagation();
        onCloseRef.current();
      }
      if (e.key === 'Tab' && panelRef.current) {
        // Keep keyboard focus inside the dialog.
        const f = focusables();
        if (!f.length) return;
        const first = f[0];
        const last = f[f.length - 1];
        const inside = panelRef.current.contains(document.activeElement);
        if (!inside || (e.shiftKey && document.activeElement === first) || (!e.shiftKey && document.activeElement === last)) {
          e.preventDefault();
          (e.shiftKey ? last : first).focus();
        }
      }
    };
    window.addEventListener('keydown', onKey);
    return () => {
      clearTimeout(t);
      window.removeEventListener('keydown', onKey);
      returnTo.current?.focus?.();
    };
  }, [open]);

  return (
    <AnimatePresence>
      {open && (
        <div className="fixed inset-0 z-40">
          <motion.div
            className="absolute inset-0 bg-[var(--scrim)] backdrop-blur-[2px] lg:bg-black/10"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.24 }}
            onClick={dismissible ? onClose : undefined}
          />
          <motion.div
            ref={panelRef}
            role="dialog"
            aria-modal="true"
            aria-labelledby={titleId}
            initial={{ y: 40, opacity: 0 }}
            animate={{ y: 0, opacity: 1 }}
            exit={{ y: 32, opacity: 0 }}
            transition={{ type: 'spring', stiffness: 380, damping: 36, mass: 0.9 }}
            className={cn(
              'glass-strong absolute flex flex-col text-fg',
              // phones/tablets: liquid-glass bottom sheet
              'inset-x-0 bottom-0 mx-auto max-h-[90dvh] w-full max-w-[580px] rounded-t-[var(--radius-sheet)]',
              tall && 'h-[90dvh]',
              // desktop: floating right-side panel
              'lg:inset-y-3 lg:right-3 lg:left-auto lg:mx-0 lg:h-auto lg:max-h-none lg:w-[420px] lg:max-w-none lg:rounded-[30px]'
            )}
          >
            <div aria-hidden="true" className="mx-auto mt-2 h-[5px] w-10 shrink-0 rounded-full bg-[var(--fg-subtle)] opacity-40 lg:hidden" />
            <header className="flex min-h-14 items-center gap-1 px-3 pt-1.5 lg:pt-3">
              {onBack ? (
                <IconButton label="Back" onClick={onBack}>
                  <ChevronLeft className="h-5 w-5" />
                </IconButton>
              ) : (
                <span className="w-2" />
              )}
              <h2 id={titleId} tabIndex={-1} className="t-title flex-1 px-1 text-heading outline-none">
                {title}
              </h2>
              {dismissible && (
                <IconButton label={closeLabel} onClick={onClose}>
                  <X className="h-[18px] w-[18px]" />
                </IconButton>
              )}
            </header>
            <div className="scroll-area flex-1 overflow-y-auto px-4 pb-4">{children}</div>
            {footer && <div className="safe-b border-t border-[var(--glass-stroke)] px-4 pt-3">{footer}</div>}
            {!footer && <div className="safe-b" />}
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  );
}
