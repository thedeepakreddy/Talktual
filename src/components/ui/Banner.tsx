import type { ReactNode } from 'react';
import { AlertCircle, Info, WifiOff } from 'lucide-react';
import { cn } from '../../utils';

export type BannerTone = 'info' | 'warning' | 'error' | 'offline';

/** The one place a problem or notice is shown. */
export function Banner({ tone = 'info', children, action }: { tone?: BannerTone; children: ReactNode; action?: ReactNode }) {
  const Icon = tone === 'offline' ? WifiOff : tone === 'info' ? Info : AlertCircle;
  return (
    <div
      role={tone === 'error' ? 'alert' : 'status'}
      className={cn(
        'flex items-start gap-3 rounded-md px-3.5 py-2.5 text-label',
        tone === 'error' ? 'bg-danger-soft text-fg' : tone === 'warning' || tone === 'offline' ? 'bg-accent-soft text-fg' : 'bg-surface text-fg'
      )}
    >
      <Icon
        aria-hidden="true"
        className={cn('mt-px h-4 w-4 shrink-0', tone === 'error' ? 'text-danger' : tone === 'info' ? 'text-fg-muted' : 'text-accent-text')}
      />
      <p className="flex-1">{children}</p>
      {action}
    </div>
  );
}
