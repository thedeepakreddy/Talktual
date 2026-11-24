import { useId } from 'react';
import { cn } from '../../utils';

export interface SwitchProps {
  label: string;
  description?: string;
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
}

/** A labelled on/off setting. The whole row is the target. Glass track; molten when on. */
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
      className="flex min-h-[56px] w-full items-center gap-4 py-3 text-left disabled:opacity-40"
    >
      <span className="min-w-0 flex-1">
        <span className="block text-body-lg font-[520] text-fg">{label}</span>
        {description && (
          <span id={descId} className="mt-0.5 block text-label text-fg-muted">
            {description}
          </span>
        )}
      </span>
      <span
        aria-hidden="true"
        className={cn(
          'relative h-[31px] w-[51px] shrink-0 rounded-full transition-[background,box-shadow] duration-300',
          checked ? 'molten !shadow-[inset_0_1px_0_rgb(255_255_255/0.55),0_0_0_1px_rgb(255_150_70/0.5),0_6px_18px_-6px_var(--glow)]' : 'bg-[var(--glass-top)] shadow-[inset_0_0_0_1px_var(--glass-stroke),inset_0_2px_4px_rgb(0_0_0/0.18)]'
        )}
      >
        <span
          className={cn(
            'absolute top-[2px] h-[27px] w-[27px] rounded-full bg-[linear-gradient(180deg,#ffffff,#f1ebe5)] shadow-[0_3px_8px_rgb(0_0_0/0.28),0_0_0_0.5px_rgb(0_0_0/0.06),inset_0_-1px_1px_rgb(0_0_0/0.06)] transition-transform duration-300 ease-[var(--ease-spring)]',
            checked ? 'translate-x-[22px]' : 'translate-x-[2px]'
          )}
        />
      </span>
    </button>
  );
}
