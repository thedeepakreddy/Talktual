import type { ReactNode } from 'react';
import { AlertCircle, Info, WifiOff } from '../icons';
import { cn } from '../../utils';

export type BannerTone = 'info' | 'warning' | 'error' | 'offline';

/** The one place a problem or notice is shown: a glass card with a tinted rim. */
export function Banner({ tone = 'info', children, action }: { tone?: BannerTone; children: ReactNode; action?: ReactNode }) {
  const Icon = tone === 'offline' ? WifiOff : tone === 'info' ? Info : AlertCircle;
  const tint = tone === 'error' ? 'var(--danger)' : tone === 'info' ? 'var(--fg-subtle)' : 'var(--accent)';
  return (
    <div
      role={tone === 'error' ? 'alert' : 'status'}
      className="glass flex items-start gap-3 rounded-[22px] px-3.5 py-3 text-label text-fg"
      style={{ background: `linear-gradient(180deg, color-mix(in oklab, ${tint} 16%, var(--glass-top)), color-mix(in oklab, ${tint} 6%, var(--glass)))` }}
    >
      <span
        aria-hidden="true"
        className={cn('flex h-7 w-7 shrink-0 items-center justify-center rounded-[10px]', tone === 'error' ? 'text-danger' : tone === 'info' ? 'text-fg-muted' : 'text-accent-text')}
        style={{ background: `color-mix(in oklab, ${tint} 18%, transparent)` }}
      >
        <Icon className="h-4 w-4" strokeWidth={1.9} />
      </span>
      <p className="flex-1 pt-[5px] leading-[1.35]">{children}</p>
      {action}
    </div>
  );
}
