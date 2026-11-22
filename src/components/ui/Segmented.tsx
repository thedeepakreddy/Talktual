import { useId, type KeyboardEvent } from 'react';
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

/** 2–3 mutually exclusive choices, as a radiogroup (arrow keys move). */
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
    <div role="radiogroup" aria-label={label} onKeyDown={onKey} className="flex rounded-md bg-surface p-1">
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
            className={cn(
              'h-11 flex-1 rounded-sm px-2 text-label font-semibold transition-colors duration-150 disabled:opacity-40',
              selected ? 'bg-selected text-fg shadow-[0_1px_2px_rgb(0_0_0/0.25)]' : 'text-fg-muted hover:text-fg'
            )}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}
