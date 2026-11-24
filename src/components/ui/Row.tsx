import type { ReactNode } from 'react';
import { ChevronRight } from '../icons';

export interface RowProps {
  label: string;
  value?: ReactNode;
  onClick?: () => void;
  ariaLabel?: string;
}

/** Navigation row inside a grouped card. */
export function Row({ label, value, onClick, ariaLabel }: RowProps) {
  return (
    <button type="button" onClick={onClick} aria-label={ariaLabel} className="group flex min-h-[56px] w-full items-center gap-3 py-3 text-left">
      <span className="flex-1 text-body-lg font-[520] text-fg">{label}</span>
      {value != null && <span className="min-w-0 truncate text-body text-fg-muted">{value}</span>}
      <ChevronRight className="h-[18px] w-[18px] shrink-0 text-fg-subtle transition-transform duration-200 group-hover:translate-x-0.5" aria-hidden="true" />
    </button>
  );
}

/** Inset-grouped glass card with an eyebrow heading. */
export function Section({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <section className="pt-7 first:pt-1">
      {title && <h3 className="t-eyebrow px-1 pb-2.5 text-fg-subtle">{title}</h3>}
      <div className="glass-clear rounded-[26px] px-4 [&>*+*]:border-t [&>*+*]:border-[var(--glass-stroke)]">{children}</div>
    </section>
  );
}
