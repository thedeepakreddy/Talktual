import type { IconComponent } from '../icons';
import { cn } from '../../utils';

/** A feature icon in a tinted glass squircle, lit from within. */
export function Glyph({ icon: Icon, size = 'md', className }: { icon: IconComponent; size?: 'md' | 'lg'; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn('relative flex items-center justify-center text-accent-text', size === 'lg' ? 'h-16 w-16 rounded-[22px]' : 'h-11 w-11 rounded-[15px]', className)}
      style={{
        background: 'linear-gradient(160deg, color-mix(in oklab, var(--accent) 30%, transparent), color-mix(in oklab, var(--accent) 8%, transparent))',
        boxShadow: 'inset 0 1px 0 var(--glass-hi), inset 0 0 0 1px color-mix(in oklab, var(--accent) 32%, transparent), 0 12px 30px -14px var(--glow)',
      }}
    >
      <Icon className={size === 'lg' ? 'h-7 w-7' : 'h-5 w-5'} strokeWidth={1.6} />
    </span>
  );
}
