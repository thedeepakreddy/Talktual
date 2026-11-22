/** Talktual mark: a transmit dot with outgoing arcs (from the original app mark). */
export function BrandMark({ size = 24 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <circle cx="9" cy="12" r="3.2" fill="var(--accent)" />
      <path d="M14 7.5a6.4 6.4 0 0 1 0 9M17.4 4.6a10.4 10.4 0 0 1 0 14.8" stroke="var(--accent)" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}

export function Wordmark() {
  return (
    <span className="inline-flex items-center gap-2">
      <BrandMark />
      <span className="text-heading font-semibold tracking-[-0.01em]">Talktual</span>
    </span>
  );
}
