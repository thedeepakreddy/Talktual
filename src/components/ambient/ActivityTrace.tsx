import { useEffect, useRef, useState } from 'react';
import { cn } from '../../utils';

const SLICE_MS = 250;
const SLICES = 60; // 15 s window

/**
 * Listening canvas: a horizontal, time-based trace of the last 15 seconds.
 * Each bar is one 250 ms slice. A slice rises only when the recognizer REPORTED
 * something in it — speech detected, or new interim words (more updates → taller).
 * It is not an audio level meter (browsers' speech recognition exposes no level),
 * and it never animates on its own: no recognizer events → flat line.
 * Paused/stopped: the trace stops advancing and settles (dims).
 */
export function ActivityTrace({ capturing, hearing, partial, settled, className }: { capturing: boolean; hearing: boolean; partial: string; settled: boolean; className?: string }) {
  const [slices, setSlices] = useState<number[]>(() => Array(SLICES).fill(0));
  const updates = useRef(0);
  const hearingRef = useRef(hearing);
  hearingRef.current = hearing;

  // Count interim-text updates as they arrive (real recognizer events).
  const lastPartial = useRef(partial);
  useEffect(() => {
    if (partial && partial !== lastPartial.current) updates.current++;
    lastPartial.current = partial;
  }, [partial]);

  useEffect(() => {
    if (!capturing) return;
    const id = setInterval(() => {
      const n = updates.current;
      updates.current = 0;
      const v = n > 0 ? Math.min(1, 0.35 + n * 0.25) : hearingRef.current ? 0.3 : 0;
      setSlices((s) => [...s.slice(1), v]);
    }, SLICE_MS);
    return () => clearInterval(id);
  }, [capturing]);

  const any = slices.some((v) => v > 0);
  const W = 600;
  const H = 48;
  const step = W / SLICES;
  return (
    <figure className={cn('m-0', className)}>
      <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" className={cn('block h-12 w-full transition-opacity duration-200', settled && 'opacity-45')} role="img" aria-label={any ? 'Speech the recognizer detected over the last 15 seconds' : 'No speech detected yet'}>
        {/* Baseline: silence is a continuous hairline, not a pattern. */}
        <rect x={0} y={H / 2 - 0.75} width={W} height={1.5} fill="var(--line-strong)" />
        {slices.map((v, i) => {
          if (v <= 0) return null;
          const h = Math.max(6, v * H);
          return <rect key={i} x={i * step + 1} y={(H - h) / 2} width={Math.max(1.5, step - 3)} height={h} rx={1.5} fill="var(--accent-text)" />;
        })}
      </svg>
      <figcaption className="mt-1 flex justify-between text-meta text-fg-subtle tnum" aria-hidden="true">
        <span>−15 s</span>
        <span>−10</span>
        <span>−5</span>
        <span>now</span>
      </figcaption>
    </figure>
  );
}
