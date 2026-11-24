import { useId, type KeyboardEvent } from 'react';
import { motion } from 'motion/react';
import { cn } from '../../utils';

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
  disabled?: boolean;
}

export interface SegmentedProps<T extends string> {
  label: string;
  options: SegmentedOption<T>[];
  value: T;
  onChange: (value: T) => void;
}

/** 2–3 mutually exclusive choices, as a radiogroup (arrow keys move). A glass lens slides to the choice. */
export function Segmented<T extends string>({ label, options, value, onChange }: SegmentedProps<T>) {
  const name = useId();
  const enabled = options.filter((o) => !o.disabled);
  const onKey = (e: KeyboardEvent) => {
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) return;
    e.preventDefault();
    const i = enabled.findIndex((o) => o.value === value);
    const next = enabled[(i + (e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 1) + enabled.length) % enabled.length];
    if (next) onChange(next.value);
  };
  return (
    <div role="radiogroup" aria-label={label} onKeyDown={onKey} className="flex rounded-full bg-[var(--glass-top)] p-1 shadow-[inset_0_0_0_1px_var(--glass-stroke),inset_0_2px_6px_rgb(0_0_0/0.12)]">
      {options.map((o) => {
        const selected = o.value === value;
        return (
          <button
            key={o.value}
            type="button"
            role="radio"
            name={name}
            aria-checked={selected}
            aria-label={o.label}
            tabIndex={selected ? 0 : -1}
            disabled={o.disabled}
            onClick={() => onChange(o.value)}
            className={cn('press relative h-11 flex-1 rounded-full px-2 text-label font-[620] tracking-[-0.005em] disabled:opacity-40', selected ? 'text-fg' : 'text-fg-muted hover:text-fg')}
          >
            {selected && (
              <motion.span
                layoutId={`seg-${name}`}
                className="glass-strong absolute inset-0 rounded-full"
                transition={{ type: 'spring', stiffness: 480, damping: 38 }}
              />
            )}
            <span className="relative">{o.label}</span>
          </button>
        );
      })}
    </div>
  );
}
