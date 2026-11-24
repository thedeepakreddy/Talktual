import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from 'react';
import { cn } from '../../utils';

export interface IconButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'aria-label'> {
  /** Accessible name — required, icons alone are not labels. */
  label: string;
  children: ReactNode;
  tone?: 'default' | 'danger' | 'active' | 'plain';
}

/** A 44 px glass disc. `plain` drops the glass for icons inside dense rows. */
export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { label, children, tone = 'default', className, type = 'button', ...rest },
  ref
) {
  return (
    <button
      ref={ref}
      type={type}
      aria-label={label}
      title={label}
      className={cn(
        'press inline-flex h-11 w-11 shrink-0 items-center justify-center rounded-full disabled:opacity-40 [&_svg]:stroke-[1.75]',
        tone === 'default' && 'glass-clear text-fg hover:brightness-110',
        tone === 'plain' && 'text-fg-muted hover:bg-[var(--glass-top)] hover:text-fg',
        tone === 'active' && 'glass-clear text-accent-text [background:linear-gradient(180deg,color-mix(in_oklab,var(--accent)_26%,transparent),color-mix(in_oklab,var(--accent)_10%,transparent))]',
        tone === 'danger' && 'glass-clear text-danger [background:linear-gradient(180deg,color-mix(in_oklab,var(--danger)_24%,transparent),color-mix(in_oklab,var(--danger)_10%,transparent))]',
        className
      )}
      {...rest}
    >
      {children}
    </button>
  );
});
