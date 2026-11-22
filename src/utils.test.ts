import { describe, it, expect } from 'vitest';
import { cn } from './utils';

describe('cn', () => {
  it('keeps a text color alongside a design-system type role', () => {
    expect(cn('bg-accent text-on-accent', 'text-body')).toBe('bg-accent text-on-accent text-body');
    expect(cn('text-label text-fg-muted')).toBe('text-label text-fg-muted');
  });
  it('still resolves real conflicts', () => {
    expect(cn('text-body', 'text-label')).toBe('text-label');
    expect(cn('text-fg', 'text-danger')).toBe('text-danger');
  });
});
