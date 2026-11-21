import { useState, useEffect, useRef, useCallback } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { Language, findLanguage } from '../types';
import { Mic, MicOff, PhoneOff, MessageSquareText, X, ArrowLeftRight, Radio, ShieldCheck } from 'lucide-react';
import { cn } from '../utils';

import { useCommunicationSession } from '../engine/useCommunicationSession';
import type { SessionDeps, SessionSnapshot } from '../engine/CommunicationSession';
import type { SegmentRecord } from '../engine/events';
import { LanguagePicker } from './LanguagePicker';
import { ConsentSheet } from './ConsentSheet';
import { VOICEMIRROR_DISCLOSURE } from '../voicemirror/disclosure';

interface CallRoomProps {
  roomCode: string;
  language: Language;
  userName: string;
  onLanguageChange: (language: Language) => void;
  onEndCall: () => void;
  /** Test/preview seam: inject engine dependencies. */
  depsFactory?: () => SessionDeps;
}

function haptic(pattern: number | number[]) {
  try {
    navigator.vibrate?.(pattern);
  } catch {
    /* unsupported */
  }
}

/** How long a "they left" / "expired" message stays before returning home. */
const END_NOTICE_MS = 1800;

export function CallRoom({ roomCode, language, userName, onLanguageChange, onEndCall, depsFactory }: CallRoomProps) {
  const { session, snapshot } = useCommunicationSession(
    { roomCode, lang: language.bcp47, userName },
    depsFactory
  );

  const [showTranscript, setShowTranscript] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const scrollRef = useRef<HTMLDivElement>(null);

  const s = snapshot;
  const phase = s?.phase ?? 'connecting';
  const connected = phase === 'connected';
  const continuous = s?.mode === 'continuous';
  const micOff = s?.activity === 'stopped';

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, [s?.segments, showTranscript]);

  useEffect(() => {
    if (!connected || !s?.connectedAt) return;
    const started = s.connectedAt;
    const tick = () => setElapsed(Math.floor((Date.now() - started) / 1000));
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, [connected, s?.connectedAt]);

  // Give the user a beat to read why the call ended.
  useEffect(() => {
    if (phase !== 'ended' || (s?.phaseReason !== 'peer-left' && s?.phaseReason !== 'expired')) return;
    const t = setTimeout(onEndCall, END_NOTICE_MS);
    return () => clearTimeout(t);
  }, [phase, s?.phaseReason, onEndCall]);

  const leave = useCallback(() => {
    session?.leave();
    onEndCall();
  }, [session, onEndCall]);

  // ---- Push to talk --------------------------------------------------------

  const canPress = connected && !micOff && Boolean(s?.features.canTalk) && s?.permission !== 'denied';

  const startTalking = useCallback(() => {
    if (!session || !canPress || continuous) return;
    haptic(18);
    session.pressTalk();
  }, [session, canPress, continuous]);

  const stopTalking = useCallback(() => {
    if (!session) return;
    if (session.getSnapshot().holding) haptic(10);
    session.releaseTalk();
  }, [session]);

  useEffect(() => {
    const isTypingTarget = (el: EventTarget | null) =>
      el instanceof HTMLElement && /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName);
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.code !== 'Space' || e.repeat || isTypingTarget(e.target)) return;
      e.preventDefault();
      startTalking();
    };
    const onKeyUp = (e: KeyboardEvent) => {
      if (e.code !== 'Space' || isTypingTarget(e.target)) return;
      e.preventDefault();
      stopTalking();
    };
    window.addEventListener('keydown', onKeyDown);
    window.addEventListener('keyup', onKeyUp);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
      window.removeEventListener('keyup', onKeyUp);
    };
  }, [startTalking, stopTalking]);

  // ---- Presentation --------------------------------------------------------

  const holding = Boolean(s?.holding);
  const playing = Boolean(s?.playing);
  const hearingVoice = Boolean(s?.hearingVoice);
  const peerName = s?.peer.name ?? null;
  const peerLanguage = findLanguage(s?.peer.lang) ?? null;
  const segments = s?.segments ?? [];

  const statusLabel = (): string | null => {
    if (phase === 'idle' || phase === 'connecting') return 'Connecting';
    if (phase === 'waiting-for-peer') return s?.phaseReason === 'peer-away' ? `Waiting for ${peerName ?? 'them'} to reconnect` : 'Waiting for them to join';
    if (phase === 'reconnecting') return `Reconnecting${s && s.reconnectAttempt > 1 ? ` (try ${s.reconnectAttempt})` : ''}…`;
    if (phase === 'ended' && s?.phaseReason === 'peer-left') return `${peerName ?? 'They'} left the channel`;
    if (phase === 'ended' && s?.phaseReason === 'expired') return 'Channel expired';
    if (phase === 'ended') return 'Call ended';
    if (phase === 'failed') return s?.phaseReason === 'connection-lost' ? 'Disconnected' : 'Connection problem';
    return null;
  };

  const ledColor = connected
    ? 'var(--green)'
    : phase === 'connecting' || phase === 'waiting-for-peer' || phase === 'reconnecting' || phase === 'idle'
      ? 'var(--amber)'
      : 'var(--red)';

  const primaryHint = micOff
    ? 'Microphone off'
    : continuous && connected
      ? s?.activity === 'paused'
        ? 'Hands-free — paused while they speak'
        : hearingVoice
          ? 'Listening…'
          : 'Hands-free — listening'
      : holding
        ? hearingVoice
          ? 'Listening…'
          : 'Speak now'
        : playing
          ? 'Receiving — press to cut in'
          : connected
            ? 'Hold to talk'
            : (statusLabel() ?? '');

  const screenText = holding || continuous ? s?.partial.local || s?.partial.peer || '' : (s?.partial.peer ?? '');
  const lastHeard = lastReceived(segments);
  const mmss = `${String(Math.floor(elapsed / 60)).padStart(2, '0')}:${String(elapsed % 60).padStart(2, '0')}`;

  return (
    <div className="mx-auto flex min-h-screen w-full max-w-md flex-col px-5">
      {/* ---- Top bar ---- */}
      <div className="safe-t flex items-center justify-between gap-3 py-3">
        <button
          onClick={leave}
          aria-label="Leave channel"
          className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full border border-[var(--red)]/25 bg-[var(--red)]/12 text-[var(--red)] transition-colors hover:bg-[var(--red)]/20"
        >
          <PhoneOff className="h-[17px] w-[17px]" />
        </button>

        <div className="flex min-w-0 flex-col items-center">
          <div className="flex items-center gap-2">
            <span className={cn('led', !connected && 'led-pulse')} style={{ background: ledColor }} aria-hidden="true" />
            <span className="font-mono text-[13px] font-semibold tracking-[0.14em] text-[var(--text-1)]">{roomCode}</span>
          </div>
          <span className="tnum mt-0.5 text-[11px] text-[var(--text-3)]">{connected ? mmss : (statusLabel() ?? '')}</span>
        </div>

        <div className="flex shrink-0 gap-2">
          <button
            onClick={() => session?.setMicEnabled(micOff)}
            aria-label={micOff ? 'Turn microphone on' : 'Turn microphone off'}
            aria-pressed={micOff}
            className={cn(
              'flex h-10 w-10 items-center justify-center rounded-full border transition-colors',
              micOff
                ? 'border-[var(--red)]/25 bg-[var(--red)]/12 text-[var(--red)]'
                : 'border-white/10 bg-white/[0.06] text-[var(--text-2)] hover:bg-white/[0.1]'
            )}
          >
            {micOff ? <MicOff className="h-[17px] w-[17px]" /> : <Mic className="h-[17px] w-[17px]" />}
          </button>
          <button
            onClick={() => setShowTranscript(true)}
            aria-label="Show transcript"
            className="relative flex h-10 w-10 items-center justify-center rounded-full border border-white/10 bg-white/[0.06] text-[var(--text-2)] transition-colors hover:bg-white/[0.1]"
          >
            <MessageSquareText className="h-[17px] w-[17px]" />
            {segments.length > 0 && (
              <span className="absolute -right-0.5 -top-0.5 flex h-[17px] min-w-[17px] items-center justify-center rounded-full bg-[var(--blue)] px-1 text-[10px] font-semibold text-white">
                {segments.length > 99 ? '99' : segments.length}
              </span>
            )}
          </button>
        </div>
      </div>

      {/* ---- Hands-free indicator: always visible while it's on ---- */}
      {continuous && (
        <div
          role="status"
          className="mb-1 flex items-center justify-between gap-3 rounded-xl border border-[var(--green)]/30 bg-[var(--green)]/10 px-3.5 py-2"
        >
          <span className="flex items-center gap-2 text-[13px] font-medium text-[#9be8b0]">
            <Radio className="h-4 w-4" aria-hidden="true" />
            {s?.capturing ? 'Hands-free: listening continuously' : 'Hands-free on (paused)'}
          </span>
          <button
            onClick={() => session?.setListeningMode('push-to-talk')}
            className="rounded-full bg-white/[0.12] px-3 py-1 text-[12px] font-semibold text-[var(--text-1)]"
          >
            Stop
          </button>
        </div>
      )}

      {/* ---- Language rail ---- */}
      <div className="flex items-center gap-2 py-2">
        <button
          onClick={() => setPickerOpen(true)}
          aria-label={`You speak ${language.name}. Change language`}
          className="flex min-w-0 flex-1 items-center gap-2.5 rounded-2xl border border-white/8 bg-white/[0.06] px-3.5 py-2.5 text-left transition-colors hover:bg-white/[0.1] active:bg-white/[0.14]"
        >
          <span className="text-[20px] leading-none">{language.flag}</span>
          <span className="min-w-0 flex-1">
            <span className="block text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--text-3)]">You speak</span>
            <span className="block truncate text-[14px] font-medium text-[var(--text-1)]">{language.native}</span>
          </span>
        </button>

        <ArrowLeftRight className="h-4 w-4 shrink-0 text-[var(--text-3)]" aria-hidden="true" />

        <div className="flex min-w-0 flex-1 items-center gap-2.5 rounded-2xl border border-white/8 bg-white/[0.03] px-3.5 py-2.5">
          <span className="text-[20px] leading-none">{peerLanguage?.flag ?? '🌐'}</span>
          <span className="min-w-0 flex-1">
            <span className="block text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--text-3)]">
              {peerName ?? 'They'} hear{peerName ? 's' : ''}
            </span>
            <span className="block truncate text-[14px] font-medium text-[var(--text-1)]">{peerLanguage?.native ?? '—'}</span>
          </span>
        </div>
      </div>

      <AnimatePresence>
        {s?.notice && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            className="overflow-hidden"
          >
            <p
              role="status"
              className="mt-1 rounded-xl border border-[var(--amber)]/25 bg-[var(--amber)]/10 px-3.5 py-2.5 text-[13px] leading-snug text-[#ffcf80]"
            >
              {s.notice.message}
            </p>
          </motion.div>
        )}
      </AnimatePresence>

      {/* ---- Speaker panel: grille + live caption readout ---- */}
      <div
        className={cn(
          'relative mt-3 flex min-h-[150px] flex-1 flex-col overflow-hidden rounded-[24px] border border-white/8 transition-colors duration-300',
          playing ? 'bg-[var(--blue)]/[0.07]' : 'bg-black/25'
        )}
      >
        <div className={cn('grille absolute inset-0', playing ? 'opacity-100' : 'opacity-45')} aria-hidden="true" />

        <div className="sheet-scroll relative flex flex-1 items-center justify-center overflow-y-auto px-6 py-6">
          <AnimatePresence>
            {screenText ? (
              <motion.p
                key="caption"
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0 }}
                className={cn('text-center text-[17px] leading-relaxed', holding ? 'text-[#ffd9a0]' : 'text-[var(--text-1)]')}
              >
                {screenText}
                <span className="opacity-50">…</span>
              </motion.p>
            ) : (
              <motion.p
                key="idle"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                className={cn(
                  'text-center',
                  // The last thing they said is content, not a hint — it stays
                  // legible so you can re-read it while composing a reply.
                  lastHeard ? 'text-[17px] leading-relaxed text-[var(--text-2)]' : 'text-[14px] leading-snug text-[var(--text-3)]'
                )}
              >
                {lastHeard ?? 'Whatever you say appears here, translated.'}
              </motion.p>
            )}
          </AnimatePresence>
        </div>
      </div>

      {/* ---- Push to talk ---- */}
      <div className="safe-b flex flex-col items-center pt-6">
        <div className="relative flex h-[212px] w-[212px] items-center justify-center">
          <AnimatePresence>
            {(holding || (continuous && s?.capturing)) && (
              <>
                <motion.span
                  key="r1"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  className="ptt-ripple absolute inset-0 rounded-full border border-[var(--amber)]/45"
                />
                <motion.span
                  key="r2"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  style={{ animationDelay: '0.65s' }}
                  className="ptt-ripple absolute inset-0 rounded-full border border-[var(--amber)]/35"
                />
              </>
            )}
            {playing && !holding && (
              <motion.span
                key="recv"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                className="ptt-ripple absolute inset-0 rounded-full border border-[var(--blue)]/45"
              />
            )}
          </AnimatePresence>

          <button
            onPointerDown={(e) => {
              startTalking();
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
              stopTalking();
            }}
            onPointerCancel={stopTalking}
            onContextMenu={(e) => e.preventDefault()}
            disabled={!canPress || continuous}
            aria-label="Push to talk"
            aria-pressed={holding}
            className={cn(
              'ptt-bezel no-touch h-[188px] w-[188px]',
              (holding || (continuous && s?.capturing)) && 'ptt-live ptt-pressed',
              playing && !holding && 'ptt-recv',
              (!canPress || continuous) && 'ptt-disabled'
            )}
          >
            <span className="ptt-face flex h-full w-full flex-col items-center justify-center gap-1.5">
              <Mic
                className={cn('h-9 w-9 transition-colors', holding ? 'text-black/80' : playing ? 'text-white' : 'text-[var(--text-2)]')}
              />
              <span
                className={cn(
                  'text-[11px] font-bold uppercase tracking-[0.14em] transition-colors',
                  holding ? 'text-black/60' : 'text-[var(--text-3)]'
                )}
              >
                {holding ? 'On air' : continuous ? 'Hands-free' : 'Hold'}
              </span>
            </span>
          </button>
        </div>

        <p
          aria-live="polite"
          className={cn(
            'mt-5 text-center text-[15px] font-medium transition-colors',
            holding ? 'text-[var(--amber)]' : playing ? 'text-[var(--blue)]' : 'text-[var(--text-2)]'
          )}
        >
          {primaryHint}
        </p>

        <div className="mt-2 flex flex-wrap items-center justify-center gap-2 text-[11px] text-[var(--text-3)]">
          <span>{s?.transport === 'p2p' ? 'Direct' : 'Relayed'}</span>
          <span aria-hidden="true">·</span>
          <span>{s?.features.stt === 'server-stt' ? 'Batch transcribe' : 'Live transcribe'}</span>
          {s?.lastLatencyMs != null && (
            <>
              <span aria-hidden="true">·</span>
              <span className="tnum">{s.lastLatencyMs}ms</span>
            </>
          )}
          {s?.voice.peerActive && (
            <>
              <span aria-hidden="true">·</span>
              <span>Their voice style matched</span>
            </>
          )}
          {s?.features.serverKnown && s.features.tts !== 'server' && (
            <>
              <span aria-hidden="true">·</span>
              <span>System voice</span>
            </>
          )}
        </div>

        {/* ---- Listening mode + disclosure ---- */}
        <div className="mt-3 flex items-center gap-3 text-[12px]">
          {s?.features.continuous && (
            <button
              onClick={() => session?.setListeningMode(continuous ? 'push-to-talk' : 'continuous')}
              disabled={!connected || micOff}
              aria-pressed={continuous}
              className="rounded-full border border-white/10 bg-white/[0.05] px-3 py-1.5 font-medium text-[var(--text-2)] transition-colors hover:bg-white/[0.1] disabled:opacity-40"
            >
              {continuous ? 'Hands-free: on' : 'Hands-free: off'}
            </button>
          )}
          {s?.voice.available && (
            <button
              onClick={() => session?.toggleVoiceMirror()}
              disabled={!connected}
              aria-pressed={s.voice.sharing}
              className="rounded-full border border-white/10 bg-white/[0.05] px-3 py-1.5 font-medium text-[var(--text-2)] transition-colors hover:bg-white/[0.1] disabled:opacity-40"
            >
              {s.voice.sharing ? 'Voice match: on' : 'Voice match: off'}
            </button>
          )}
          <button
            onClick={() => session?.showDisclosure()}
            className="flex items-center gap-1 text-[var(--text-3)] underline-offset-2 hover:text-[var(--text-2)] hover:underline"
          >
            <ShieldCheck className="h-3.5 w-3.5" aria-hidden="true" />
            How your voice is processed
          </button>
        </div>
      </div>

      {/* ---- Transcript ---- */}
      <AnimatePresence>
        {showTranscript && (
          <>
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              onClick={() => setShowTranscript(false)}
              className="fixed inset-0 z-40 bg-black/60 backdrop-blur-[2px]"
            />
            <motion.div
              role="dialog"
              aria-modal="true"
              aria-label="Transcript"
              initial={{ y: '100%' }}
              animate={{ y: 0 }}
              exit={{ y: '100%' }}
              transition={{ type: 'spring', damping: 32, stiffness: 340 }}
              className="fixed inset-x-0 bottom-0 z-50 mx-auto flex h-[80vh] w-full max-w-md flex-col overflow-hidden rounded-t-[28px] border border-white/10 bg-[#141418]/95 backdrop-blur-2xl"
            >
              <div className="flex justify-center pt-3 pb-1">
                <div className="h-1 w-9 rounded-full bg-white/20" />
              </div>
              <div className="flex items-center justify-between px-5 pt-2 pb-4">
                <h2 className="text-[19px] font-semibold tracking-[-0.01em]">Transcript</h2>
                <button
                  onClick={() => setShowTranscript(false)}
                  aria-label="Close transcript"
                  className="flex h-8 w-8 items-center justify-center rounded-full bg-white/8 text-[var(--text-2)] transition-colors hover:bg-white/14"
                >
                  <X className="h-4 w-4" />
                </button>
              </div>

              <div ref={scrollRef} className="sheet-scroll flex-1 space-y-4 overflow-y-auto px-5 pb-6">
                {segments.length === 0 ? (
                  <p className="pt-16 text-center text-[15px] text-[var(--text-3)]">Nothing said yet.</p>
                ) : (
                  segments.map((seg) => <TranscriptBubble key={seg.id} seg={seg} />)
                )}
              </div>
            </motion.div>
          </>
        )}
      </AnimatePresence>

      <LanguagePicker
        open={pickerOpen}
        selected={language}
        title="You speak"
        subtitle="Switching takes effect immediately"
        onSelect={onLanguageChange}
        onClose={() => setPickerOpen(false)}
      />

      <ConsentSheet
        open={Boolean(s?.voice.prompt)}
        disclosure={VOICEMIRROR_DISCLOSURE}
        granted={false}
        allowLabel="Share voice style"
        onAllow={() => session?.acceptVoiceMirror()}
        onDecline={() => session?.declineVoiceMirror()}
      />

      <ConsentSheet
        open={Boolean(s?.consent.prompt)}
        disclosure={s?.consent.disclosure ?? null}
        granted={Boolean(s?.consent.granted)}
        onAllow={() => session?.grantConsent()}
        onDecline={() => session?.declineConsent()}
      />
    </div>
  );
}

function TranscriptBubble({ seg }: { seg: SegmentRecord; key?: string }) {
  const isMe = seg.speaker === 'local';
  const secondary = seg.untranslated
    ? 'Not translated — translation was unavailable'
    : isMe
      ? seg.translation === 'pending'
        ? 'Translating…'
        : seg.translated
      : seg.original;
  return (
    <div className={cn('flex flex-col', isMe ? 'items-end' : 'items-start')}>
      <span className="mb-1 px-1 text-[11px] font-medium text-[var(--text-3)]">{isMe ? 'You' : seg.speakerName || 'Them'}</span>
      <div
        className={cn(
          'max-w-[85%] rounded-[20px] px-4 py-3',
          isMe ? 'rounded-br-[6px] bg-[var(--blue)] text-white' : 'rounded-bl-[6px] bg-white/[0.09] text-[var(--text-1)]'
        )}
      >
        <p className="text-[16px] leading-snug">{isMe ? seg.original : seg.translated}</p>
        <p className={cn('mt-1.5 text-[13px] leading-snug', isMe ? 'text-white/65' : 'text-[var(--text-3)]')}>{secondary}</p>
      </div>
    </div>
  );
}

/** The last thing the other person said, shown on the idle panel. */
function lastReceived(segments: SessionSnapshot['segments']): string | null {
  for (let i = segments.length - 1; i >= 0; i--) {
    if (segments[i].speaker === 'peer') return segments[i].translated;
  }
  return null;
}
