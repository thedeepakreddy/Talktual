import { clsx, type ClassValue } from 'clsx';
import { extendTailwindMerge } from 'tailwind-merge';

// The design system's type roles (text-meta … text-title) are font sizes, not
// colors. Without this, tailwind-merge treats them as colors and silently drops
// the real color class (e.g. `text-on-accent` on the primary button).
const twMerge = extendTailwindMerge({
  extend: { classGroups: { 'font-size': [{ text: ['meta', 'label', 'body', 'body-lg', 'heading', 'title'] }] } },
});

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
