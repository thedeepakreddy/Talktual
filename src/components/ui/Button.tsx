import { forwardRef, type ButtonHTMLAttributes, type PointerEvent, type ReactNode } from 'react';
import { cn } from '../../utils';

type Variant = 'primary' | 'secondary' | 'tertiary' | 'ghost' | 'danger';
type Size = 'md' | 'sm' | 'lg';

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
  icon?: ReactNode;
  /** Trailing icon in its own lens (e.g. an arrow on the main call to action). */
  chip?: ReactNode;
  block?: boolean;
}

const VARIANT: Record<Variant, string> = {
  // Molten: specular bloom, Cosmic body, ember base, liquid cap, pointer sheen.
  primary: 'molten',
  // Prism glass: dark glass, specular top edge, warm rim on hover.
  secondary: 'glass-strong prism press text-fg hover:brightness-110',
  // Ember line: gradient outline, ember text.
  tertiary: 'ember-line',
  ghost: 'press text-fg-muted hover:bg-[var(--glass-top)] hover:text-fg',
  danger: 'glass press text-danger [background:linear-gradient(180deg,color-mix(in_oklab,var(--danger)_22%,transparent),color-mix(in_oklab,var(--danger)_10%,transparent))]',
};
const SIZE: Record<Size, string> = {
  lg: 'h-14 px-7 text-body-lg font-[620] gap-2.5',
  md: 'h-[52px] px-6 text-body font-[620] gap-2',
  sm: 'h-11 px-4 text-label font-[620] gap-1.5',
};

/** Pointer position for the molten sheen. */
function sheen(e: PointerEvent<HTMLButtonElement>) {
  const r = e.currentTarget.getBoundingClientRect();
  e.currentTarget.style.setProperty('--mx', `${e.clientX - r.left}px`);
  e.currentTarget.style.setProperty('--my', `${e.clientY - r.top}px`);
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = 'secondary', size = 'md', icon, chip, block, className, children, type = 'button', onPointerMove, ...rest },
  ref
) {
  return (
    <button
      ref={ref}
      type={type}
      onPointerMove={(e) => {
        if (variant === 'primary') sheen(e);
        onPointerMove?.(e);
      }}
      className={cn(
        'inline-flex select-none items-center justify-center whitespace-nowrap rounded-full tracking-[-0.01em]',
        'disabled:cursor-not-allowed disabled:opacity-40 disabled:saturate-50',
        VARIANT[variant],
        SIZE[size],
        block && 'w-full',
        className
      )}
      {...rest}
    >
      {icon && variant === 'primary' ? <span className="btn-chip -ml-2.5 [&_svg]:h-[18px] [&_svg]:w-[18px]">{icon}</span> : icon}
      {children}
      {chip && <span className={cn(variant === 'primary' ? 'btn-chip' : 'btn-chip-glass', '-mr-3 ml-1 [&_svg]:h-4 [&_svg]:w-4')}>{chip}</span>}
    </button>
  );
});
