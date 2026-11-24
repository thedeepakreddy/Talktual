import { useState, useEffect, useCallback, useRef } from 'react';
import { PhoneOff, SlidersHorizontal, X } from './icons';
import { Language, findLanguage } from '../types';
import { cn } from '../utils';

import { useCommunicationSession } from '../engine/useCommunicationSession';
import type { SessionDeps } from '../engine/CommunicationSession';
import { LanguagePicker } from './LanguagePicker';
import { ConsentSheet } from './ConsentSheet';
import { VOICEMIRROR_DISCLOSURE } from '../voicemirror/disclosure';
import { Banner, Button, IconButton, Sheet } from './ui';
import { ConversationFeed } from './call/ConversationFeed';
import { TalkDock } from './call/TalkDock';
import { VoiceWave } from './voice/VoiceWave';
import { createLevelMeter, liveMicStream } from '../lib/audio';
import { SettingsSheet } from './call/SettingsSheet';
import { deriveStatus, headerState } from './call/status';

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
  const { session, snapshot: s } = useCommunicationSession({ roomCode, lang: language.bcp47, userName }, depsFactory);

  const [settingsOpen, setSettingsOpen] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [confirmEnd, setConfirmEnd] = useState(false);
  const [typing, setTyping] = useState(false);
  const [draft, setDraft] = useState('');
  const [elapsed, setElapsed] = useState(0);
  const composerRef = useRef<HTMLInputElement>(null);

  const phase = s?.phase ?? 'connecting';
  const connected = phase === 'connected';
  const continuous = s?.mode === 'continuous';
  const micOff = s?.activity === 'stopped';
  // Real loudness for the voice wave only where the call already holds a mic stream.
  const micMeter = useCallback((onLevel: (v: number) => void) => {
    const stream = liveMicStream();
    return stream ? createLevelMeter(stream, onLevel) : null;
  }, []);

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

  // A consent prompt always takes over: never stack it on top of settings.
  useEffect(() => {
    if (s?.voice.prompt || s?.consent.prompt) setSettingsOpen(false);
  }, [s?.voice.prompt, s?.consent.prompt]);

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
      el instanceof HTMLElement && /^(INPUT|TEXTAREA|SELECT|BUTTON)$/.test(el.tagName) && el.getAttribute('aria-label') !== 'Push to talk';
    const sheetOpen = () => document.querySelector('[role="dialog"]') !== null;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.code !== 'Space' || e.repeat || isTypingTarget(e.target) || sheetOpen()) return;
      e.preventDefault();
      startTalking();
    };
    const onKeyUp = (e: KeyboardEvent) => {
      if (e.code !== 'Space' || isTypingTarget(e.target) || sheetOpen()) return;
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
  const peerName = s?.peer.name ?? null;
  const peerLabel = peerName ?? 'They';
  const peerLanguage = findLanguage(s?.peer.lang) ?? null;
  const status = deriveStatus(s);
  const speechUnavailable = s?.processing.available && s.processing.capabilities.find((c) => c.task === 'speech-input')?.state === 'unavailable';
  const showComposer = Boolean(s?.processing.available) && connected && (typing || speechUnavailable);
  const mmss = `${String(Math.floor(elapsed / 60)).padStart(2, '0')}:${String(elapsed % 60).padStart(2, '0')}`;
  const mode = s?.processing.available ? s.processing.mode : 'cloud';
  const suggestion = s?.intelligence.suggestions.at(-1);

  const dot = connected ? 'bg-success' : phase === 'failed' || phase === 'ended' ? 'bg-danger' : 'bg-accent';

  return (
    // On desktop, sheets are a 400 px side panel: the call column moves aside so
    // the conversation stays readable while settings are open.
    <div className={cn('transition-[padding] duration-300', (settingsOpen || pickerOpen) && 'lg:pr-[432px]')}>
      <div className="mx-auto flex h-dvh w-full max-w-[780px] flex-col">
        {/* ---- Header: who, connection, settings, end ---- */}
        <header className="safe-t px-3 pt-2 sm:px-5">
          <div className="glass-strong @container flex min-h-[64px] items-center gap-2.5 rounded-[30px] py-2 pl-2.5 pr-2">
          <span aria-hidden="true" className="orb-core hidden h-11 w-11 shrink-0 items-center justify-center rounded-full text-body-lg font-[680] text-on-accent @min-[19rem]:flex">
            {(peerName ?? '·').slice(0, 1).toUpperCase()}
          </span>
          <div className="min-w-0 flex-1">
            <p className="truncate text-body-lg font-[640] tracking-[-0.015em]">{peerName ?? (connected ? 'Connected' : 'Waiting for them')}</p>
            <p className="flex items-center gap-1.5 text-label text-fg-muted" data-testid="call-meta">
              <span className={cn('h-2 w-2 shrink-0 rounded-full', dot, connected && 'shadow-[0_0_8px_var(--success)]')} aria-hidden="true" />
              <span className="truncate">
                {connected
                  ? [
                      `${language.native} ⇄ ${peerLanguage?.native ?? '…'}`,
                      mode === 'local-only' ? 'On-device only' : mode === 'hybrid' ? 'Hybrid' : null,
                      s?.voice.peerActive ? 'Voice matched' : null,
                      mmss,
                    ]
                      .filter(Boolean)
                      .join(' · ')
                  : headerState(s, peerLabel)}
              </span>
            </p>
          </div>
          <IconButton label="Call settings" onClick={() => setSettingsOpen(true)}>
            <SlidersHorizontal className="h-[18px] w-[18px]" />
          </IconButton>
          <Button
            variant="danger"
            size="sm"
            // A live call asks first (a mis-tap would drop the other person); once the call is over, End just leaves.
            onClick={connected || phase === 'reconnecting' || phase === 'waiting-for-peer' ? () => setConfirmEnd(true) : leave}
            aria-label="End call"
            icon={<PhoneOff className="h-4 w-4" />}
            className="shrink-0 px-3.5 @min-[20rem]:px-4"
          >
            <span className="hidden @min-[20rem]:inline">End</span>
          </Button>
          </div>
        </header>

        {/* ---- Persistent hands-free indicator (privacy: always visible while on) + one banner ---- */}
        {(continuous || status.banner) && (
          <div className="space-y-2 px-4 pt-3 sm:px-6">
            {continuous && (
              <Banner
                tone="info"
                action={
                  <button
                    className="-my-3 -mr-2 min-h-11 px-2 text-label font-semibold text-accent-text"
                    onClick={() => session?.setListeningMode('push-to-talk')}
                  >
                    Stop
                  </button>
                }
              >
                {s?.capturing ? 'Hands-free: listening continuously' : 'Hands-free on (paused)'}
              </Banner>
            )}
            {status.banner && (
              <Banner
                tone={status.banner.tone}
                action={
                  status.banner.action === 'leave' ? (
                    <button className="-my-3 -mr-2 min-h-11 px-2 text-label font-semibold text-danger" onClick={leave}>
                      Leave
                    </button>
                  ) : status.banner.action === 'type' && !typing ? (
                    <button className="-my-3 -mr-2 min-h-11 px-2 text-label font-semibold text-accent-text" onClick={() => setTyping(true)}>
                      Type
                    </button>
                  ) : undefined
                }
              >
                {status.banner.text}
              </Banner>
            )}
          </div>
        )}

        {/* ---- The conversation ---- */}
        <ConversationFeed snapshot={s} peerName={peerLabel} peerLanguage={peerLanguage?.native ?? null} />

        {/* ---- Phase 5 suggestion: one at a time, dismissible, never acts on its own ---- */}
        {suggestion && (
          <div role="note" aria-label="Suggestion" className="glass mx-4 mb-2 flex items-start gap-2 rounded-[22px] px-3.5 py-2.5 sm:mx-6">
            <p className="flex-1 text-label text-fg-muted">{suggestion.text}</p>
            <button
              onClick={() => session?.dismissSuggestion(suggestion.id)}
              aria-label="Dismiss suggestion"
              className="-my-3 -mr-3 flex h-11 w-11 shrink-0 items-center justify-center rounded-sm text-fg-subtle hover:text-fg"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        )}

        {/* ---- Typed input (always offered when speech recognition is unavailable) ---- */}
        {showComposer && (
          <form
            className="flex gap-2 px-4 pb-1 sm:px-6"
            onSubmit={(e) => {
              e.preventDefault();
              session?.sendText(draft);
              setDraft('');
              composerRef.current?.focus();
            }}
          >
            <input
              ref={composerRef}
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              maxLength={500}
              aria-label="Type a message"
              placeholder="Type a message"
              className="glass-clear h-12 min-w-0 flex-1 rounded-full px-5 text-body text-fg outline-none placeholder:text-fg-subtle focus:shadow-[inset_0_0_0_1.5px_var(--focus)]"
            />
            <Button type="submit" variant="primary" size="sm" className="h-12 px-5" disabled={!draft.trim()}>
              Send
            </Button>
          </form>
        )}

        {connected && !speechUnavailable && (
          <VoiceWave
            capturing={Boolean(s?.capturing)}
            hearing={Boolean(s?.hearingVoice)}
            partial={s?.partial.local ?? ''}
            settled={micOff}
            meter={micMeter}
            className="h-24 sm:h-28"
          />
        )}

        <TalkDock
          hint={status.hint}
          tone={status.tone}
          canPress={canPress && !continuous}
          holding={Boolean(s?.holding)}
          continuous={continuous}
          capturing={Boolean(s?.capturing)}
          hearingVoice={Boolean(s?.hearingVoice)}
          playing={Boolean(s?.playing)}
          micOff={micOff}
          typing={typing}
          showTypeToggle={Boolean(s?.processing.available) && connected && !speechUnavailable}
          onPressStart={startTalking}
          onPressEnd={stopTalking}
          onStopHandsFree={() => session?.setListeningMode('push-to-talk')}
          onToggleMic={() => session?.setMicEnabled(micOff)}
          onToggleTyping={() => {
            setTyping((v) => !v);
            setTimeout(() => composerRef.current?.focus(), 30);
          }}
        />

        <SettingsSheet
          open={settingsOpen}
          onClose={() => setSettingsOpen(false)}
          session={session}
          snapshot={s}
          roomCode={roomCode}
          language={language}
          onChangeLanguage={() => {
            setSettingsOpen(false);
            setPickerOpen(true);
          }}
          onShowDisclosure={() => {
            setSettingsOpen(false);
            session?.showDisclosure();
          }}
        />

        <Sheet
          open={confirmEnd}
          onClose={() => setConfirmEnd(false)}
          title="End this call?"
          footer={
            <div className="grid grid-cols-2 gap-3">
              <Button onClick={() => setConfirmEnd(false)} data-autofocus>
                Keep talking
              </Button>
              <Button variant="danger" onClick={leave}>
                End call
              </Button>
            </div>
          }
        >
          <p className="pb-1 text-body text-fg-muted">{peerName ? `${peerName} will be disconnected.` : 'The other person will be disconnected.'} The conversation is not saved.</p>
        </Sheet>

        <LanguagePicker open={pickerOpen} selected={language} title="You speak" onSelect={onLanguageChange} onClose={() => setPickerOpen(false)} />

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
    </div>
  );
}
