import { Fragment, useEffect, useRef } from 'react';
import { Volume2 } from '../icons';
import type { SegmentRecord } from '../../engine/events';
import type { SessionSnapshot } from '../../engine/CommunicationSession';
import { cn } from '../../utils';

/** Plain-language state for a line (Phase 4–6 markers), shown as quiet metadata. */
export function markersFor(seg: SegmentRecord): string[] {
  const m: string[] = [];
  if (seg.state === 'provisional') m.push(seg.speaker === 'peer' ? 'Still speaking — may change' : 'Draft');
  if (seg.interrupted) m.push(seg.speaker === 'peer' ? 'Interrupted — not played' : 'Interrupted');
  if (seg.overlapHeld && seg.playback !== 'started' && seg.playback !== 'ended' && !seg.interrupted) {
    m.push(seg.playback === 'dropped' ? 'Skipped — arrived too late' : 'Waiting until you finish');
  } else if (seg.playback === 'dropped') m.push('Skipped — arrived too late');
  if (seg.uncertain) m.push('Unclear audio — may be inaccurate');
  if (seg.repaired && seg.speaker === 'local') m.push('Tidied repeats');
  if (seg.assisted) m.push('Context-assisted');
  if (seg.typed) m.push('Typed');
  if (seg.processedOn === 'device') m.push('Translated on this device');
  if (seg.privateRequest) m.push('They asked for on-device processing');
  if (seg.correctedBy) m.push('Corrected later');
  return m;
}

/** Signals worth surfacing (others stay in diagnostics). */
const SIGNAL_TAG: Record<string, string> = {
  question: 'Question',
  request: 'Request',
  offer: 'Offer',
  correction: 'Correction',
  urgency: 'Urgent',
  'clarification-request': 'Asks to clarify',
  misunderstanding: 'Possible misunderstanding',
};

interface FeedProps {
  snapshot: SessionSnapshot | null;
  peerName: string;
  peerLanguage: string | null;
}

export function ConversationFeed({ snapshot: s, peerName, peerLanguage }: FeedProps) {
  const ref = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const segments = s?.segments ?? [];
  const held = s?.conversation.held ?? null;
  const live = s?.holding || s?.mode === 'continuous' ? s?.partial.local || held?.text || '' : '';
  const peerLive = s?.partial.peer ?? '';

  // Follow new content unless the user scrolled up to re-read.
  useEffect(() => {
    const el = ref.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [segments, live, peerLive]);

  const empty = segments.length === 0 && !live && !peerLive;
  const connected = s?.phase === 'connected';

  return (
    <div
      ref={ref}
      role="log"
      aria-label="Conversation"
      onScroll={(e) => {
        const el = e.currentTarget;
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
      }}
      className="scroll-area flex-1 overflow-y-auto px-4 sm:px-6"
    >
      {empty ? (
        <div className="flex h-full min-h-48 items-center justify-center py-10">
          <p className="max-w-[19rem] text-center text-body-lg text-fg-muted">
            {connected
              ? `Hold the button and speak. ${peerName} hears you${peerLanguage ? ` in ${peerLanguage}` : ''}.`
              : s?.phase === 'waiting-for-peer' || s?.phase === 'connecting' || !s
                ? 'The conversation will appear here.'
                : ''}
          </p>
        </div>
      ) : (
        <ol className="flex min-h-full flex-col justify-end py-4">
          {segments.map((seg, i) => {
            const prev = segments[i - 1];
            const showName = !prev || prev.speaker !== seg.speaker;
            return (
              <Fragment key={seg.id}>
                <Turn seg={seg} showName={showName} peerName={peerName} latest={i === segments.length - 1} />
              </Fragment>
            );
          })}
          {peerLive && (
            <li className="py-3" aria-label={`${peerName}, speaking`}>
              <p className="text-body-lg italic text-fg-muted">
                {peerLive}
                <span aria-hidden="true">…</span>
              </p>
            </li>
          )}
          {live && (
            <li className="flex flex-col items-end py-3" aria-label="You, speaking">
              <p className="max-w-[85%] rounded-[22px] rounded-br-[8px] px-4 py-2.5 text-body-lg text-accent-text shadow-[inset_0_0_0_1px_color-mix(in_oklab,var(--accent)_35%,transparent)]" style={{ background: 'color-mix(in oklab, var(--accent) 10%, transparent)' }}>
                {live}
                <span aria-hidden="true">…</span>
              </p>
              {held && !s?.holding && <p className="mt-1 text-meta text-fg-subtle">Waiting for the rest of your sentence</p>}
            </li>
          )}
        </ol>
      )}
    </div>
  );
}

function Turn({ seg, showName, peerName, latest }: { seg: SegmentRecord; showName: boolean; peerName: string; latest: boolean }) {
  const mine = seg.speaker === 'local';
  const markers = markersFor(seg);
  // Signals help the listener read intent; on your own lines they're noise.
  const tags = mine ? [] : (seg.signals ?? []).map((x) => SIGNAL_TAG[x.category]).filter(Boolean);
  const provisional = seg.state === 'provisional';
  const playing = seg.playback === 'started';
  const primary = mine ? seg.original : seg.translated || seg.original;
  const secondary = mine
    ? seg.untranslated
      ? 'Not translated — translation was unavailable'
      : seg.translation === 'pending'
        ? 'Translating…'
        : seg.translated
    : seg.untranslated
      ? 'Not translated'
      : seg.original !== primary
        ? seg.original
        : '';

  return (
    <li className={cn('flex flex-col', mine ? 'items-end' : 'items-start', showName ? 'pt-5' : 'pt-1.5', 'pb-1')}>
      {showName &&
        (mine ? (
          <span className="mb-1.5 px-1 text-label font-[600] tracking-[0.01em] text-fg-subtle">You</span>
        ) : (
          <span className="mb-2 flex items-center gap-2">
            <span aria-hidden="true" className="orb-core h-[18px] w-[18px] rounded-full" />
            <span className="text-label font-[600] tracking-[0.01em] text-fg-muted">{seg.speakerName || peerName}</span>
          </span>
        ))}
      <div
        className={cn(
          mine ? 'glass-strong max-w-[85%] rounded-[24px] rounded-br-[8px] px-4 py-3' : 'max-w-[40rem] pl-[26px]',
          (provisional || seg.interrupted) && 'opacity-70'
        )}
        style={mine ? { background: 'linear-gradient(160deg, color-mix(in oklab, var(--accent) 26%, var(--glass-strong-top)), color-mix(in oklab, var(--accent) 12%, var(--glass-strong)))' } : undefined}
        data-state={seg.state ?? 'final'}
      >
        <p className={cn('text-fg', !mine && latest ? 'text-translation font-[600] tracking-[-0.018em]' : 'text-body-lg', provisional && 'italic')}>
          {playing && <Volume2 className="mr-1.5 inline h-4 w-4 align-[-2px] text-incoming" aria-label="Playing" />}
          {primary}
        </p>
        {secondary && <p className={cn('mt-1 text-label', mine ? 'text-fg-muted' : 'text-fg-muted')}>{secondary}</p>}
        {(markers.length > 0 || tags.length > 0) && <p className="mt-1.5 text-meta text-fg-subtle">{[...tags, ...markers].join(' · ')}</p>}
        {seg.notes?.length ? (
          <ul className="mt-1.5 space-y-0.5 text-label text-fg-muted" data-testid="aci-notes">
            {seg.notes.map((n, i) => (
              <li key={i}>{n.text}</li>
            ))}
          </ul>
        ) : null}
      </div>
    </li>
  );
}
