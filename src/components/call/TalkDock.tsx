import { AudioLines, Keyboard, Mic, MicOff, Square } from '../icons';
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
    <div className="safe-b px-3 pt-1 sm:px-6">
      <p aria-live="polite" className={cn('mb-2.5 min-h-5 text-center text-label font-[560] transition-colors', TONE_TEXT[p.tone])}>
        {p.hint}
        {p.tone === 'idle' && p.hint === 'Hold to talk' && <span className="hidden text-fg-subtle [@media(hover:hover)]:inline"> · or hold Space</span>}
      </p>
      <div className="glass-strong mx-auto grid max-w-[400px] grid-cols-[1fr_auto_1fr] items-center rounded-[40px] px-4 py-2.5">
        <div className="flex justify-start">
          <span className="flex flex-col items-center">
            <IconButton label={p.micOff ? 'Turn microphone on' : 'Turn microphone off'} aria-pressed={p.micOff} tone={p.micOff ? 'danger' : 'default'} onClick={p.onToggleMic}>
              {p.micOff ? <MicOff className="h-5 w-5" /> : <Mic className="h-5 w-5" />}
            </IconButton>
            <span className="mt-1 text-[0.6875rem] font-[560] text-fg-subtle" aria-hidden="true">
              {p.micOff ? 'Mic off' : 'Mic'}
            </span>
          </span>
        </div>

        {p.continuous ? (
          <button
            type="button"
            onClick={p.onStopHandsFree}
            aria-label="Stop hands-free"
            className="glass-clear press relative flex h-[88px] w-[88px] items-center justify-center rounded-full"
          >
            <span className="orb-core absolute inset-[6px] rounded-full" aria-hidden="true" />
            <span className="relative flex flex-col items-center gap-0.5 text-on-accent">
              {live && p.hearingVoice ? <Bars /> : <Square className="h-5 w-5 fill-current" />}
              <span className="text-[0.6875rem] font-[680]">Stop</span>
            </span>
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
              'no-touch glass-clear relative flex h-[88px] w-[88px] items-center justify-center rounded-full transition-[transform,box-shadow] duration-200 ease-[var(--ease-spring)]',
              'disabled:cursor-not-allowed disabled:opacity-40 disabled:saturate-50',
              live && 'scale-[0.96] shadow-[0_0_0_6px_color-mix(in_oklab,var(--accent)_22%,transparent),0_0_44px_-4px_var(--glow)]',
              !live && p.playing && 'shadow-[0_0_0_2px_var(--incoming)]'
            )}
          >
            <span aria-hidden="true" className={cn('orb-core absolute inset-[6px] rounded-full transition-transform duration-200', live && 'scale-[0.97] brightness-110')} />
            <span className="relative text-on-accent">{live && p.hearingVoice ? <Bars /> : <Mic className="h-8 w-8" strokeWidth={2} />}</span>
          </button>
        )}

        <div className="flex justify-end">
          {p.showTypeToggle && (
            <span className="flex flex-col items-center">
              <IconButton label="Type" aria-pressed={p.typing} tone={p.typing ? 'active' : 'default'} onClick={p.onToggleTyping}>
                <Keyboard className="h-5 w-5" />
              </IconButton>
              <span className="mt-1 text-[0.6875rem] font-[560] text-fg-subtle" aria-hidden="true">
                Type
              </span>
            </span>
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
