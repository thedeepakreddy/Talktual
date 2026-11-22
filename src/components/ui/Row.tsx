import type { ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';

export interface RowProps {
  label: string;
  value?: ReactNode;
  onClick?: () => void;
  ariaLabel?: string;
}

/** Navigation row inside a sheet. */
export function Row({ label, value, onClick, ariaLabel }: RowProps) {
  return (
    <button type="button" onClick={onClick} aria-label={ariaLabel} className="flex min-h-12 w-full items-center gap-3 py-2.5 text-left">
      <span className="flex-1 text-body text-fg">{label}</span>
      {value != null && <span className="truncate text-body text-fg-muted">{value}</span>}
      <ChevronRight className="h-4 w-4 shrink-0 text-fg-subtle" aria-hidden="true" />
    </button>
  );
}

/** Group heading + divider-separated children. */
export function Section({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <section className="pt-5 first:pt-1">
      {title && <h3 className="pb-1 text-label font-semibold text-fg-muted">{title}</h3>}
      <div className="divide-y divide-line">{children}</div>
    </section>
  );
}
