import { useState, useEffect, useRef, useCallback } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { Message, Language, findLanguage } from '../types';
import { Mic, MicOff, PhoneOff, MessageSquareText, X, ArrowLeftRight } from 'lucide-react';
import { cn } from '../utils';

import { connectCall, CallHandle, CallMessage, Transport } from '../lib/webrtc';
import { createRecognizer, Recognizer, SttEngine } from '../lib/speechToText';
import { translateText, getCapabilities, prewarm, Capabilities } from '../lib/translate';
import { enqueueSpeech, stopSpeaking, isSpeaking, preloadVoices } from '../lib/textToSpeech';
import { unlockAudio, releaseMic } from '../lib/audio';
import { SIGNALING_URL } from '../lib/config';
import { LanguagePicker } from './LanguagePicker';

type CallStatus =
  | 'waiting-for-peer'
  | 'connecting'
  | 'connected'
  | 'peer-left'
  | 'expired'
  | 'error'
  | 'disconnected';

interface CallRoomProps {
  roomCode: string;
  language: Language;
  userName: string;
  onLanguageChange: (language: Language) => void;
  onEndCall: () => void;
}

const PARTIAL_SEND_INTERVAL_MS = 180;
const PARTIAL_TRANSLATE_DEBOUNCE_MS = 400;

function newId() {
  return crypto.randomUUID?.() ?? Math.random().toString(36).slice(2);
}

function haptic(pattern: number | number[]) {
  try {
    navigator.vibrate?.(pattern);
  } catch {
    /* unsupported */
  }
}

export function CallRoom({
  roomCode,
  language,
  userName,
  onLanguageChange,
  onEndCall,
}: CallRoomProps) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [status, setStatus] = useState<CallStatus>('waiting-for-peer');
  const [transport, setTransport] = useState<Transport>('relay');
  const [holding, setHolding] = useState(false);
  const [hearingVoice, setHearingVoice] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [muted, setMuted] = useState(false);
  const [myPartial, setMyPartial] = useState('');
  const [theirPartial, setTheirPartial] = useState('');
  const [peerName, setPeerName] = useState<string | null>(null);
  const [peerLanguage, setPeerLanguage] = useState<Language | null>(null);
  const [engine, setEngine] = useState<SttEngine>('webspeech');
  const [caps, setCaps] = useState<Capabilities | null>(null);
  const [lastLatencyMs, setLastLatencyMs] = useState<number | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [showTranscript, setShowTranscript] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [myId] = useState(newId);

  const callRef = useRef<CallHandle | null>(null);
  const recognizerRef = useRef<Recognizer | null>(null);
  const peerLangRef = useRef<string | null>(null);
  const capsRef = useRef<Capabilities | null>(null);
  const holdingRef = useRef(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  // The call outlives any single render, so anything it reads back must come
  // from a ref — otherwise switching language mid-call would need a reconnect.
  const langRef = useRef(language);
  langRef.current = language;
  const nameRef = useRef(userName);
  nameRef.current = userName;

  // Translations are awaited, so two segments could finish out of order.
  // Chaining them preserves the order the words were actually spoken in.
  const sendChain = useRef<Promise<void>>(Promise.resolve());
  const lastPartialSentAt = useRef(0);
  const partialTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const partialAbort = useRef<AbortController | null>(null);

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, [messages, showTranscript]);

  useEffect(() => {
    if (status !== 'connected') return;
    const started = Date.now();
    const id = setInterval(() => setElapsed(Math.floor((Date.now() - started) / 1000)), 1000);
    return () => clearInterval(id);
  }, [status]);

  // Give the user a beat to read why the call ended.
  useEffect(() => {
    if (status !== 'peer-left' && status !== 'expired') return;
    const t = setTimeout(onEndCall, 1800);
    return () => clearTimeout(t);
  }, [status, onEndCall]);

  // ---- Outgoing: one finalized phrase -> translate -> ship ------------------

  const handleSegment = useCallback((text: string) => {
    const id = newId();
    const sourceLang = langRef.current.bcp47;
    const targetLang = peerLangRef.current ?? sourceLang;
    const startedAt = performance.now();

    sendChain.current = sendChain.current
      .then(async () => {
        const translated = await translateText(text, sourceLang, targetLang);
        const latencyMs = Math.round(performance.now() - startedAt);

        callRef.current?.send({
          t: 'segment',
          id,
          original: text,
          translated,
          sourceLang,
          targetLang,
          sentAt: Date.now(),
        });

        setLastLatencyMs(latencyMs);
        setMessages((prev) => [
          ...prev,
          {
            id,
            speakerId: myId,
            speakerName: nameRef.current,
            originalText: text,
            translatedText: translated,
            sourceLang,
            targetLang,
            latencyMs,
          },
        ]);
      })
      .catch((err) => console.error('segment pipeline failed:', err));
  }, [myId]);

  const handlePartial = useCallback((text: string) => {
    setMyPartial(text);
    const now = Date.now();
    if (!text || now - lastPartialSentAt.current < PARTIAL_SEND_INTERVAL_MS) return;
    lastPartialSentAt.current = now;
    callRef.current?.send({ t: 'partial', text, lang: langRef.current.bcp47 });
  }, []);

  // ---- Incoming ------------------------------------------------------------

  const handleMessage = useCallback((msg: CallMessage) => {
    const myLang = langRef.current.bcp47;

    if (msg.t === 'hello') {
      peerLangRef.current = msg.lang;
      setPeerLanguage(findLanguage(msg.lang) ?? null);
      setPeerName(msg.name || null);
      return;
    }

    if (msg.t === 'partial') {
      if (!msg.text) {
        setTheirPartial('');
        return;
      }
      // Without a fast translator, showing raw source text is more honest than
      // burning a rate-limited quota on text that changes every 200ms.
      if (!capsRef.current?.liveCaptions) {
        setTheirPartial(msg.text);
        return;
      }
      if (partialTimer.current) clearTimeout(partialTimer.current);
      partialTimer.current = setTimeout(async () => {
        partialAbort.current?.abort();
        const controller = new AbortController();
        partialAbort.current = controller;
        try {
          setTheirPartial(await translateText(msg.text, msg.lang, myLang, controller.signal));
        } catch {
          /* superseded by a newer partial */
        }
      }, PARTIAL_TRANSLATE_DEBOUNCE_MS);
      return;
    }

    if (msg.t === 'segment') {
      setTheirPartial('');
      if (partialTimer.current) clearTimeout(partialTimer.current);

      const deliver = (text: string) => {
        setMessages((prev) => [
          ...prev,
          {
            id: msg.id,
            speakerId: 'peer',
            speakerName: peerName ?? 'Them',
            originalText: msg.original,
            translatedText: text,
            sourceLang: msg.sourceLang,
            targetLang: myLang,
          },
        ]);
        haptic(12);
        enqueueSpeech(text, myLang, {
          onStart: () => setPlaying(true),
          onEnd: () => setPlaying(isSpeaking()),
        });
      };

      // The sender normally translates into our language using the handshake.
      // If their hello hadn't landed yet — or we just switched language — fix
      // it up on this side.
      if (msg.targetLang.split('-')[0] === myLang.split('-')[0]) {
        deliver(msg.translated);
      } else {
        translateText(msg.original, msg.sourceLang, myLang)
          .then(deliver)
          .catch(() => deliver(msg.translated));
      }
    }
  }, [peerName]);

  // Keep callbacks reachable from the long-lived recognizer/call instances.
  const handleSegmentRef = useRef(handleSegment);
  const handlePartialRef = useRef(handlePartial);
  const handleMessageRef = useRef(handleMessage);
  handleSegmentRef.current = handleSegment;
  handlePartialRef.current = handlePartial;
  handleMessageRef.current = handleMessage;

  // ---- The call itself: created once, survives language changes -------------

  useEffect(() => {
    preloadVoices();
    prewarm();

    getCapabilities().then((c) => {
      capsRef.current = c;
      setCaps(c);
    });

    const call = connectCall(roomCode, SIGNALING_URL, {
      onMessage: (msg) => handleMessageRef.current(msg),
      onStatus: (s) => setStatus(s as CallStatus),
      onTransport: setTransport,
    });
    callRef.current = call;
    call.send({ t: 'hello', lang: langRef.current.bcp47, name: nameRef.current });

    return () => {
      if (partialTimer.current) clearTimeout(partialTimer.current);
      partialAbort.current?.abort();
      stopSpeaking();
      releaseMic();
      call.close();
      callRef.current = null;
    };
  }, [roomCode]);

  // ---- Recognition: rebuilt when the language (or mute) changes -------------

  useEffect(() => {
    if (muted) return;

    const recognizer = createRecognizer(language.bcp47, {
      onSegment: (text) => handleSegmentRef.current(text),
      onPartial: (text) => handlePartialRef.current(text),
      onSpeechActivity: (active) => setHearingVoice(active && holdingRef.current),
      onError: (message, fatal) => {
        setNotice(message);
        if (!fatal) setTimeout(() => setNotice(null), 5000);
      },
    });
    recognizerRef.current = recognizer;
    setEngine(recognizer.engine);
    recognizer.arm().catch((err) => setNotice(String(err)));

    return () => {
      recognizer.destroy();
      recognizerRef.current = null;
    };
  }, [language.bcp47, muted]);

  // Re-announce whenever our language changes so the peer retargets instantly.
  useEffect(() => {
    if (status === 'connected') {
      callRef.current?.send({ t: 'hello', lang: language.bcp47, name: userName });
    }
  }, [status, language.bcp47, userName]);

  // ---- Push to talk --------------------------------------------------------

  const canTalk = status === 'connected' && !muted;

  const startTalking = useCallback(() => {
    if (!canTalk || holdingRef.current) return;
    if (isSpeaking()) {
      stopSpeaking();
      setPlaying(false);
    }
    holdingRef.current = true;
    setHolding(true);
    setMyPartial('');
    haptic(18);
    void unlockAudio();
    recognizerRef.current?.hold();
  }, [canTalk]);

  const stopTalking = useCallback(() => {
    if (!holdingRef.current) return;
    holdingRef.current = false;
    setHolding(false);
    setHearingVoice(false);
    haptic(10);
    recognizerRef.current?.release();
    callRef.current?.send({ t: 'partial', text: '', lang: langRef.current.bcp47 });
  }, []);

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

  const statusLabel = () => {
    if (status === 'waiting-for-peer') return 'Waiting for them to join';
    if (status === 'connecting') return 'Connecting';
    if (status === 'peer-left') return `${peerName ?? 'They'} left the channel`;
    if (status === 'expired') return 'Channel expired';
    if (status === 'error') return 'Connection problem';
    if (status === 'disconnected') return 'Disconnected';
    return null;
  };

  const ledColor =
    status === 'connected'
      ? 'var(--green)'
      : status === 'connecting' || status === 'waiting-for-peer'
        ? 'var(--amber)'
        : 'var(--red)';

  const primaryHint = muted
    ? 'Microphone off'
    : holding
      ? hearingVoice
        ? 'Listening…'
        : 'Speak now'
      : playing
        ? 'Receiving — press to cut in'
        : status === 'connected'
          ? 'Hold to talk'
          : (statusLabel() ?? '');

  const screenText = holding ? myPartial : theirPartial;
  const lastHeard = lastReceived(messages, myId);
  const mmss = `${String(Math.floor(elapsed / 60)).padStart(2, '0')}:${String(elapsed % 60).padStart(2, '0')}`;

  return (
    <div className="mx-auto flex min-h-screen w-full max-w-md flex-col px-5">
      {/* ---- Top bar ---- */}
      <div className="safe-t flex items-center justify-between gap-3 py-3">
        <button
          onClick={onEndCall}
          aria-label="Leave channel"
          className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full border border-[var(--red)]/25 bg-[var(--red)]/12 text-[var(--red)] transition-colors hover:bg-[var(--red)]/20"
        >
          <PhoneOff className="h-[17px] w-[17px]" />
        </button>

        <div className="flex min-w-0 flex-col items-center">
          <div className="flex items-center gap-2">
            <span
              className={cn('led', status !== 'connected' && 'led-pulse')}
              style={{ background: ledColor }}
              aria-hidden="true"
            />
            <span className="font-mono text-[13px] font-semibold tracking-[0.14em] text-[var(--text-1)]">
              {roomCode}
            </span>
          </div>
          <span className="tnum mt-0.5 text-[11px] text-[var(--text-3)]">
            {status === 'connected' ? mmss : (statusLabel() ?? '')}
          </span>
        </div>

        <div className="flex shrink-0 gap-2">
          <button
            onClick={() => {
              setMuted((v) => !v);
              if (!muted) stopTalking();
            }}
            aria-label={muted ? 'Turn microphone on' : 'Turn microphone off'}
            className={cn(
              'flex h-10 w-10 items-center justify-center rounded-full border transition-colors',
              muted
                ? 'border-[var(--red)]/25 bg-[var(--red)]/12 text-[var(--red)]'
                : 'border-white/10 bg-white/[0.06] text-[var(--text-2)] hover:bg-white/[0.1]'
            )}
          >
            {muted ? <MicOff className="h-[17px] w-[17px]" /> : <Mic className="h-[17px] w-[17px]" />}
          </button>
          <button
            onClick={() => setShowTranscript(true)}
            aria-label="Show transcript"
            className="relative flex h-10 w-10 items-center justify-center rounded-full border border-white/10 bg-white/[0.06] text-[var(--text-2)] transition-colors hover:bg-white/[0.1]"
          >
            <MessageSquareText className="h-[17px] w-[17px]" />
            {messages.length > 0 && (
              <span className="absolute -right-0.5 -top-0.5 flex h-[17px] min-w-[17px] items-center justify-center rounded-full bg-[var(--blue)] px-1 text-[10px] font-semibold text-white">
                {messages.length > 99 ? '99' : messages.length}
              </span>
            )}
          </button>
        </div>
      </div>

      {/* ---- Language rail ---- */}
      <div className="flex items-center gap-2 py-2">
        <button
          onClick={() => setPickerOpen(true)}
          className="flex min-w-0 flex-1 items-center gap-2.5 rounded-2xl border border-white/8 bg-white/[0.06] px-3.5 py-2.5 text-left transition-colors hover:bg-white/[0.1] active:bg-white/[0.14]"
        >
          <span className="text-[20px] leading-none">{language.flag}</span>
          <span className="min-w-0 flex-1">
            <span className="block text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--text-3)]">
              You speak
            </span>
            <span className="block truncate text-[14px] font-medium text-[var(--text-1)]">
              {language.native}
            </span>
          </span>
        </button>

        <ArrowLeftRight className="h-4 w-4 shrink-0 text-[var(--text-3)]" aria-hidden="true" />

        <div className="flex min-w-0 flex-1 items-center gap-2.5 rounded-2xl border border-white/8 bg-white/[0.03] px-3.5 py-2.5">
          <span className="text-[20px] leading-none">{peerLanguage?.flag ?? '🌐'}</span>
          <span className="min-w-0 flex-1">
            <span className="block text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--text-3)]">
              {peerName ?? 'They'} hear{peerName ? 's' : ''}
            </span>
            <span className="block truncate text-[14px] font-medium text-[var(--text-1)]">
              {peerLanguage?.native ?? '—'}
            </span>
          </span>
        </div>
      </div>

      <AnimatePresence>
        {notice && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            className="overflow-hidden"
          >
            <p className="mt-1 rounded-xl border border-[var(--amber)]/25 bg-[var(--amber)]/10 px-3.5 py-2.5 text-[13px] leading-snug text-[#ffcf80]">
              {notice}
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
        <div
          className={cn('grille absolute inset-0', playing ? 'opacity-100' : 'opacity-45')}
          aria-hidden="true"
        />

        <div className="sheet-scroll relative flex flex-1 items-center justify-center overflow-y-auto px-6 py-6">
          <AnimatePresence>
            {screenText ? (
              <motion.p
                key="caption"
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0 }}
                className={cn(
                  'text-center text-[17px] leading-relaxed',
                  holding ? 'text-[#ffd9a0]' : 'text-[var(--text-1)]'
                )}
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
                  lastHeard
                    ? 'text-[17px] leading-relaxed text-[var(--text-2)]'
                    : 'text-[14px] leading-snug text-[var(--text-3)]'
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
            {holding && (
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
                if (e.currentTarget.hasPointerCapture(e.pointerId)) {
                  e.currentTarget.releasePointerCapture(e.pointerId);
                }
              } catch {
                /* nothing to release */
              }
              stopTalking();
            }}
            onPointerCancel={stopTalking}
            onContextMenu={(e) => e.preventDefault()}
            disabled={!canTalk}
            aria-label="Push to talk"
            className={cn(
              'ptt-bezel no-touch h-[188px] w-[188px]',
              holding && 'ptt-live ptt-pressed',
              playing && !holding && 'ptt-recv',
              !canTalk && 'ptt-disabled'
            )}
          >
            <span className="ptt-face flex h-full w-full flex-col items-center justify-center gap-1.5">
              <Mic
                className={cn(
                  'h-9 w-9 transition-colors',
                  holding ? 'text-black/80' : playing ? 'text-white' : 'text-[var(--text-2)]'
                )}
              />
              <span
                className={cn(
                  'text-[11px] font-bold uppercase tracking-[0.14em] transition-colors',
                  holding ? 'text-black/60' : 'text-[var(--text-3)]'
                )}
              >
                {holding ? 'On air' : 'Hold'}
              </span>
            </span>
          </button>
        </div>

        <p
          className={cn(
            'mt-5 text-center text-[15px] font-medium transition-colors',
            holding ? 'text-[var(--amber)]' : playing ? 'text-[var(--blue)]' : 'text-[var(--text-2)]'
          )}
        >
          {primaryHint}
        </p>

        <div className="mt-2 flex items-center gap-2 text-[11px] text-[var(--text-3)]">
          <span>{transport === 'p2p' ? 'Direct' : 'Relayed'}</span>
          <span aria-hidden="true">·</span>
          <span>{engine === 'webspeech' ? 'Live transcribe' : 'Batch transcribe'}</span>
          {lastLatencyMs !== null && (
            <>
              <span aria-hidden="true">·</span>
              <span className="tnum">{lastLatencyMs}ms</span>
            </>
          )}
          {caps && !caps.tts && (
            <>
              <span aria-hidden="true">·</span>
              <span>System voice</span>
            </>
          )}
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
                {messages.length === 0 ? (
                  <p className="pt-16 text-center text-[15px] text-[var(--text-3)]">
                    Nothing said yet.
                  </p>
                ) : (
                  messages.map((msg) => {
                    const isMe = msg.speakerId === myId;
                    return (
                      <div
                        key={msg.id}
                        className={cn('flex flex-col', isMe ? 'items-end' : 'items-start')}
                      >
                        <span className="mb-1 px-1 text-[11px] font-medium text-[var(--text-3)]">
                          {isMe ? 'You' : msg.speakerName || 'Them'}
                        </span>
                        <div
                          className={cn(
                            'max-w-[85%] rounded-[20px] px-4 py-3',
                            isMe
                              ? 'rounded-br-[6px] bg-[var(--blue)] text-white'
                              : 'rounded-bl-[6px] bg-white/[0.09] text-[var(--text-1)]'
                          )}
                        >
                          <p className="text-[16px] leading-snug">
                            {isMe ? msg.originalText : msg.translatedText}
                          </p>
                          <p
                            className={cn(
                              'mt-1.5 text-[13px] leading-snug',
                              isMe ? 'text-white/65' : 'text-[var(--text-3)]'
                            )}
                          >
                            {isMe ? msg.translatedText : msg.originalText}
                          </p>
                        </div>
                      </div>
                    );
                  })
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
    </div>
  );
}

/** The last thing the other person said, shown on the idle panel. */
function lastReceived(messages: Message[], myId: string): string | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].speakerId !== myId) return messages[i].translatedText;
  }
  return null;
}
