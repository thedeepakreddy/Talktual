import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from 'react';
import { cn } from '../../utils';

type Variant = 'primary' | 'secondary' | 'ghost' | 'danger';
type Size = 'md' | 'sm';

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
  icon?: ReactNode;
  block?: boolean;
}

const VARIANT: Record<Variant, string> = {
  primary: 'bg-accent text-on-accent hover:brightness-105 active:brightness-95',
  secondary: 'bg-surface text-fg hover:bg-surface-hover border border-line',
  ghost: 'text-fg-muted hover:text-fg hover:bg-surface',
  danger: 'bg-danger-soft text-danger hover:brightness-110',
};
const SIZE: Record<Size, string> = {
  md: 'h-12 px-5 text-body font-semibold gap-2 rounded-md',
  sm: 'h-10 px-4 text-label font-semibold gap-1.5 rounded-sm',
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = 'secondary', size = 'md', icon, block, className, children, type = 'button', ...rest },
  ref
) {
  return (
    <button
      ref={ref}
      type={type}
      className={cn(
        'inline-flex select-none items-center justify-center whitespace-nowrap transition-[background-color,filter,color] duration-150',
        'disabled:cursor-not-allowed disabled:opacity-40',
        VARIANT[variant],
        SIZE[size],
        block && 'w-full',
        className
      )}
      {...rest}
    >
      {icon}
      {children}
    </button>
  );
});
