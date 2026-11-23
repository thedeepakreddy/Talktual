import { AudioLines, Keyboard, Mic, MicOff, Square } from 'lucide-react';
import { IconButton } from '../ui';
import { cn } from '../../utils';
import type { StatusTone } from './status';

interface TalkDockProps {
  hint: string;
  tone: StatusTone;
  canPress: boolean;
  holding: boolean;
  /** Hands-free is on: the main control stops it. */
  continuous: boolean;
  capturing: boolean;
  hearingVoice: boolean;
  playing: boolean;
  micOff: boolean;
  typing: boolean;
  showTypeToggle: boolean;
  onPressStart: () => void;
  onPressEnd: () => void;
  onStopHandsFree: () => void;
  onToggleMic: () => void;
  onToggleTyping: () => void;
}

const TONE_TEXT: Record<StatusTone, string> = {
  idle: 'text-fg-muted',
  live: 'text-accent-text',
  incoming: 'text-incoming',
  muted: 'text-fg-muted',
  problem: 'text-danger',
};

export function TalkDock(p: TalkDockProps) {
  const live = p.holding || (p.continuous && p.capturing);
  return (
    <div className="safe-b px-4 pt-2 sm:px-6">
      <p aria-live="polite" className={cn('mb-3 min-h-5 text-center text-label font-medium transition-colors', TONE_TEXT[p.tone])}>
        {p.hint}
        {p.tone === 'idle' && p.hint === 'Hold to talk' && <span className="hidden text-fg-subtle [@media(hover:hover)]:inline"> · or hold Space</span>}
      </p>
      <div className="mx-auto grid max-w-[360px] grid-cols-[1fr_auto_1fr] items-center">
        <div className="flex justify-start">
          <IconButton label={p.micOff ? 'Turn microphone on' : 'Turn microphone off'} aria-pressed={p.micOff} tone={p.micOff ? 'danger' : 'default'} onClick={p.onToggleMic}>
            {p.micOff ? <MicOff className="h-5 w-5" /> : <Mic className="h-5 w-5" />}
          </IconButton>
        </div>

        {p.continuous ? (
          <button
            type="button"
            onClick={p.onStopHandsFree}
            aria-label="Stop hands-free"
            className="flex h-[88px] w-[88px] flex-col items-center justify-center gap-1 rounded-full bg-accent text-on-accent transition-transform active:scale-95"
          >
            {live && p.hearingVoice ? <Bars /> : <Square className="h-6 w-6 fill-current" />}
            <span className="text-meta font-semibold">Stop</span>
          </button>
        ) : (
          <button
            type="button"
            onPointerDown={(e) => {
              p.onPressStart();
              try {
                e.currentTarget.setPointerCapture(e.pointerId);
              } catch {
                /* capture unavailable; the hold still works */
              }
            }}
            onPointerUp={(e) => {
              try {
                if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
              } catch {
                /* nothing to release */
              }
              p.onPressEnd();
            }}
            onPointerCancel={p.onPressEnd}
            onContextMenu={(e) => e.preventDefault()}
            disabled={!p.canPress}
            aria-label="Push to talk"
            aria-pressed={p.holding}
            className={cn(
              'no-touch flex h-[88px] w-[88px] items-center justify-center rounded-full transition-[transform,background-color,box-shadow] duration-150',
              'disabled:cursor-not-allowed disabled:opacity-40',
              live
                ? 'scale-95 bg-accent text-on-accent'
                : p.playing
                  ? 'bg-surface text-incoming shadow-[inset_0_0_0_2px_var(--incoming)]'
                  : 'bg-surface text-fg shadow-[inset_0_0_0_1px_var(--line-strong)] hover:bg-surface-hover'
            )}
          >
            {live && p.hearingVoice ? <Bars /> : <Mic className="h-8 w-8" />}
          </button>
        )}

        <div className="flex justify-end">
          {p.showTypeToggle && (
            <IconButton label="Type" aria-pressed={p.typing} tone={p.typing ? 'active' : 'default'} onClick={p.onToggleTyping}>
              <Keyboard className="h-5 w-5" />
            </IconButton>
          )}
        </div>
      </div>
    </div>
  );
}

/** Speech detected (static glyph: the recognizer reports speech on/off, not a level). */
function Bars() {
  return <AudioLines className="h-8 w-8" aria-hidden="true" />;
}
