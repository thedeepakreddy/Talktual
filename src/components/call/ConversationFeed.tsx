import { Fragment, useEffect, useRef } from 'react';
import { Volume2 } from 'lucide-react';
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
          <p className="max-w-[18rem] text-center text-body text-fg-muted">
            {connected
              ? `Hold the button and speak. ${peerName} hears you${peerLanguage ? ` in ${peerLanguage}` : ''}.`
              : s?.phase === 'waiting-for-peer' || s?.phase === 'connecting' || !s
                ? 'The conversation will appear here.'
                : ''}
          </p>
        </div>
      ) : (
        <ol className="flex min-h-full flex-col justify-end gap-5 py-6">
          {segments.map((seg, i) => {
            const prev = segments[i - 1];
            const showName = !prev || prev.speaker !== seg.speaker;
            return (
              <Fragment key={seg.id}>
                <Turn seg={seg} showName={showName} peerName={peerName} />
              </Fragment>
            );
          })}
          {peerLive && (
            <li className="max-w-[85%]" aria-label={`${peerName}, speaking`}>
              <p className="text-body-lg italic text-fg-muted">
                {peerLive}
                <span aria-hidden="true">…</span>
              </p>
            </li>
          )}
          {live && (
            <li className="ml-auto max-w-[85%] text-right" aria-label="You, speaking">
              <p className="text-body-lg text-accent-text">
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

function Turn({ seg, showName, peerName }: { seg: SegmentRecord; showName: boolean; peerName: string }) {
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
    <li className={cn('flex flex-col', mine ? 'items-end' : 'items-start')}>
      {showName && <span className="mb-1 text-meta font-medium text-fg-subtle">{mine ? 'You' : seg.speakerName || peerName}</span>}
      <div
        className={cn(
          'max-w-[85%] sm:max-w-[75%]',
          mine ? 'rounded-lg rounded-br-sm bg-surface px-3.5 py-2.5' : 'border-l-2 pl-3',
          !mine && (playing ? 'border-incoming' : 'border-line'),
          (provisional || seg.interrupted) && 'opacity-70'
        )}
        data-state={seg.state ?? 'final'}
      >
        <p className={cn('text-body-lg text-fg', provisional && 'italic')}>
          {playing && <Volume2 className="mr-1.5 inline h-4 w-4 align-[-2px] text-incoming" aria-label="Playing" />}
          {primary}
        </p>
        {secondary && <p className="mt-1 text-label text-fg-muted">{secondary}</p>}
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
