import { useId } from 'react';
import { cn } from '../../utils';

export interface SwitchProps {
  label: string;
  description?: string;
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
}

/** A labelled on/off setting. The whole row is the target. */
export function Switch({ label, description, checked, onChange, disabled }: SwitchProps) {
  const descId = useId();
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      aria-describedby={description ? descId : undefined}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className="flex min-h-12 w-full items-center gap-4 py-2.5 text-left disabled:opacity-40"
    >
      <span className="min-w-0 flex-1">
        <span className="block text-body text-fg">{label}</span>
        {description && (
          <span id={descId} className="mt-0.5 block text-label text-fg-muted">
            {description}
          </span>
        )}
      </span>
      <span
        aria-hidden="true"
        className={cn('relative h-7 w-12 shrink-0 rounded-full transition-colors duration-200', checked ? 'bg-accent' : 'bg-line-strong')}
      >
        <span
          className={cn(
            'absolute top-0.5 h-6 w-6 rounded-full bg-white shadow-sm transition-transform duration-200',
            checked ? 'translate-x-[22px]' : 'translate-x-0.5'
          )}
        />
      </span>
    </button>
  );
}
