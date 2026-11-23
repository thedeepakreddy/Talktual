import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from 'react';
import { cn } from '../../utils';

export interface IconButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'aria-label'> {
  /** Accessible name — required, icons alone are not labels. */
  label: string;
  children: ReactNode;
  tone?: 'default' | 'danger' | 'active';
}

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
        'inline-flex h-11 w-11 shrink-0 items-center justify-center rounded-md transition-colors duration-150 disabled:opacity-40',
        tone === 'default' && 'text-fg-muted hover:bg-surface hover:text-fg',
        tone === 'active' && 'bg-surface text-fg',
        tone === 'danger' && 'bg-danger-soft text-danger',
        className
      )}
      {...rest}
    >
      {children}
    </button>
  );
});
