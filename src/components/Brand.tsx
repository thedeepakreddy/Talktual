import { useId } from 'react';

/**
 * Talktual mark (vector rebuild of the brand sheet): two mirrored wave forms
 * split by a horizontal line — the upper with its large crest on the left,
 * the lower with its deep trough on the right. Primary → secondary orange.
 * `mono` renders one colour (currentColor) for the monochrome lockup.
 */
export function BrandMark({ size = 28, mono = false }: { size?: number; mono?: boolean }) {
  const id = useId().replace(/:/g, '');
  const fill = mono ? 'currentColor' : `url(#tt-${id})`;
  return (
    <svg width={size * 2} height={size} viewBox="0 0 96 48" fill="none" aria-hidden="true">
      {!mono && (
        <defs>
          <linearGradient id={`tt-${id}`} x1="0" y1="0" x2="96" y2="0" gradientUnits="userSpaceOnUse">
            <stop offset="0" stopColor="#E2560C" />
            <stop offset="0.5" stopColor="#FF7A1A" />
            <stop offset="1" stopColor="#FFB070" />
          </linearGradient>
        </defs>
      )}
      {/* upper wave: large crest, then a small crest */}
      <path d="M1 22.5C11 22.5 15 1 29.5 1C44 1 47 22.5 57 22.5C63.5 22.5 65.5 7.5 75 7.5C84.5 7.5 86.5 22.5 95 22.5Z" fill={fill} />
      {/* lower wave: small trough, then a large trough */}
      <path d="M1 25.5C9.5 25.5 12 40.5 21.5 40.5C31 40.5 33.5 25.5 40 25.5C50 25.5 53 47 67.5 47C82 47 85 25.5 95 25.5Z" fill={fill} />
    </svg>
  );
}

export function Wordmark({ size = 'md' }: { size?: 'md' | 'lg' }) {
  return (
    <span className="inline-flex items-center gap-2.5">
      <BrandMark size={size === 'lg' ? 26 : 20} />
      <span className={size === 'lg' ? 'text-title font-[680] tracking-[-0.035em]' : 'text-heading font-[680] tracking-[-0.03em]'}>Talktual</span>
    </span>
  );
}
