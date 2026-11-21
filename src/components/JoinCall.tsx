import { useEffect, useRef, useState } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { Scanner } from '@yudiel/react-qr-scanner';
import { ArrowLeft, Loader2 } from 'lucide-react';
import { Language } from '../types';
import { LanguagePicker } from './LanguagePicker';
import { cn } from '../utils';
import { SIGNALING_URL } from '../lib/config';
import { CODE_LENGTH, codeFromScan, normalizeRoomCode } from '../lib/roomCode';

interface JoinCallProps {
  language: Language;
  onBack: () => void;
  onJoined: (roomCode: string, lang: Language) => void;
  initialCode?: string;
}

type JoinMode = 'scan' | 'manual';

/**
 * How long to wait for the host before giving up. The host is already sitting
 * in the lobby, so a valid code pairs almost instantly; silence means the code
 * is wrong or the host left.
 */
export const JOIN_TIMEOUT_MS = 12_000;

export function JoinCall({
  language: initialLanguage,
  onBack,
  onJoined,
  initialCode = '',
}: JoinCallProps) {
  const [code, setCode] = useState(() => normalizeRoomCode(initialCode));
  const [mode, setMode] = useState<JoinMode>(initialCode.length === CODE_LENGTH ? 'manual' : 'scan');
  const [language, setLanguage] = useState(initialLanguage);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cameraFailed, setCameraFailed] = useState(false);

  const isConnectingRef = useRef(false);
  const hiddenInputRef = useRef<HTMLInputElement>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mountedRef = useRef(true);

  // Leaving this screen mid-join must close the lobby socket. Otherwise it
  // keeps a seat in that lobby and a later 'ready' would yank the user into a
  // call from whatever screen they're on.
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
      const ws = wsRef.current;
      wsRef.current = null;
      if (ws) {
        ws.onmessage = null;
        ws.onclose = null;
        ws.onerror = null;
        ws.close();
      }
    };
  }, []);

  useEffect(() => {
    if (initialCode.length === CODE_LENGTH) setMode('manual');
  }, [initialCode]);

  const handleJoin = (joinCode: string = code) => {
    if (isConnectingRef.current || joinCode.length !== CODE_LENGTH) return;

    isConnectingRef.current = true;
    setConnecting(true);
    setError(null);

    const codeUpper = joinCode.toUpperCase();
    const ws = new WebSocket(SIGNALING_URL);
    wsRef.current = ws;
    let settled = false;

    const finish = () => {
      settled = true;
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
      if (wsRef.current === ws) wsRef.current = null;
      isConnectingRef.current = false;
      ws.onmessage = null;
      ws.onclose = null;
      ws.onerror = null;
      ws.close();
    };

    const fail = (message: string) => {
      if (settled) return;
      finish();
      if (!mountedRef.current) return;
      setConnecting(false);
      setError(message);
    };

    timeoutRef.current = setTimeout(
      () => fail('No one is waiting on that channel. Check the code and try again.'),
      JOIN_TIMEOUT_MS
    );

    ws.onopen = () => {
      ws.send(JSON.stringify({ type: 'join', sessionId: codeUpper + '_signal' }));
    };

    ws.onerror = () => fail('Could not reach the server. Check your connection and try again.');

    ws.onmessage = (event) => {
      let msg: { type?: unknown; code?: unknown };
      try {
        msg = JSON.parse(event.data);
      } catch {
        return; /* ignore malformed frames */
      }
      if (msg?.type === 'ready') {
        if (settled) return;
        finish();
        if (mountedRef.current) onJoined(codeUpper, language);
      } else if (msg?.type === 'error') {
        fail(
          msg.code === 'session_full'
            ? 'That channel is already full.'
            : 'That code isn’t valid. Check it and try again.'
        );
      }
    };

    ws.onclose = () => fail('Lost connection to the server. Try again.');
  };

  const handleScan = (detectedCodes: { rawValue: string }[]) => {
    if (!detectedCodes?.length) return;
    const scanned = codeFromScan(detectedCodes[0].rawValue);
    if (scanned) handleJoin(scanned);
  };

  const setModeSafely = (next: JoinMode) => {
    setError(null);
    setMode(next);
    if (next === 'manual') setTimeout(() => hiddenInputRef.current?.focus(), 150);
  };

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.35 }}
      className="mx-auto flex min-h-screen w-full max-w-md flex-col px-6"
    >
      <div className="safe-t flex items-center justify-between py-4">
        <button
          onClick={onBack}
          aria-label="Back"
          className="flex h-10 w-10 items-center justify-center rounded-full border border-white/10 bg-white/[0.06] text-[var(--text-1)] transition-colors hover:bg-white/[0.1]"
        >
          <ArrowLeft className="h-[18px] w-[18px]" />
        </button>
        <button
          onClick={() => setPickerOpen(true)}
          className="flex items-center gap-2 rounded-full border border-white/8 bg-white/[0.05] px-3 py-1.5 transition-colors hover:bg-white/[0.1]"
        >
          <span className="text-[15px] leading-none">{language.flag}</span>
          <span className="text-[13px] font-medium text-[var(--text-2)]">{language.native}</span>
        </button>
      </div>

      <div className="pt-2 pb-6">
        <h1 className="text-[26px] font-bold tracking-[-0.02em] text-[var(--text-1)]">
          Join a channel
        </h1>
        <p className="mt-1.5 text-[15px] text-[var(--text-2)]">
          You’ll hear everything in {language.native}.
        </p>
      </div>

      {/* Segmented control */}
      <div className="mb-6 flex rounded-xl bg-white/[0.06] p-1">
        {(['scan', 'manual'] as JoinMode[]).map((value) => (
          <button
            key={value}
            onClick={() => setModeSafely(value)}
            className={cn(
              'relative flex-1 rounded-[9px] py-2 text-[14px] font-medium transition-colors',
              mode === value ? 'text-[var(--text-1)]' : 'text-[var(--text-2)]'
            )}
          >
            {mode === value && (
              <motion.span
                layoutId="join-segment"
                transition={{ type: 'spring', damping: 30, stiffness: 400 }}
                className="absolute inset-0 rounded-[9px] bg-white/[0.12]"
              />
            )}
            <span className="relative">{value === 'scan' ? 'Scan code' : 'Enter code'}</span>
          </button>
        ))}
      </div>

      <div className="flex flex-1 flex-col items-center justify-center">
        {mode === 'scan' ? (
          <div className="w-full">
            <div className="relative mx-auto aspect-square w-full max-w-[300px] overflow-hidden rounded-[28px] border border-white/10 bg-black">
              {!cameraFailed ? (
                <Scanner
                  onScan={handleScan}
                  onError={() => setCameraFailed(true)}
                  classNames={{ container: 'w-full h-full object-cover' }}
                  components={{ finder: false }}
                />
              ) : (
                <div className="flex h-full flex-col items-center justify-center gap-3 px-8 text-center">
                  <p className="text-[15px] text-[var(--text-2)]">
                    Camera unavailable.
                  </p>
                  <button
                    onClick={() => setModeSafely('manual')}
                    className="rounded-full bg-white/[0.12] px-4 py-2 text-[14px] font-medium text-[var(--text-1)]"
                  >
                    Enter the code instead
                  </button>
                </div>
              )}

              {!cameraFailed && <ScannerFrame />}
            </div>
            <p className="mt-5 text-center text-[13px] text-[var(--text-3)]">
              Point at the QR code on their screen
            </p>
          </div>
        ) : (
          <div className="w-full">
            <button
              onClick={() => hiddenInputRef.current?.focus()}
              className="flex w-full justify-center gap-1.5"
              aria-label="Enter channel code"
            >
              {Array.from({ length: CODE_LENGTH }).map((_, i) => {
                const char = code[i];
                const active = i === code.length;
                return (
                  <span
                    key={i}
                    className={cn(
                      'tnum flex h-14 w-11 items-center justify-center rounded-xl border font-mono text-[22px] font-semibold transition-colors',
                      char
                        ? 'border-white/16 bg-white/[0.1] text-[var(--text-1)]'
                        : active
                          ? 'border-[var(--blue)]/70 bg-white/[0.06] text-[var(--text-1)]'
                          : 'border-white/8 bg-white/[0.04] text-[var(--text-3)]'
                    )}
                  >
                    {char ?? ''}
                  </span>
                );
              })}
            </button>

            <input
              ref={hiddenInputRef}
              value={code}
              onChange={(e) => {
                const next = normalizeRoomCode(e.target.value);
                setCode(next);
                setError(null);
                if (next.length === CODE_LENGTH) handleJoin(next);
              }}
              inputMode="text"
              autoComplete="one-time-code"
              autoCapitalize="characters"
              autoCorrect="off"
              spellCheck={false}
              maxLength={CODE_LENGTH}
              className="pointer-events-none absolute h-px w-px opacity-0"
              aria-label="Channel code"
            />

            <p className="mt-5 text-center text-[13px] text-[var(--text-3)]">
              The 6 characters shown on their screen
            </p>
          </div>
        )}

        <AnimatePresence>
          {error && (
            <motion.p
              role="alert"
              initial={{ opacity: 0, y: -4 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              className="mt-5 rounded-xl border border-[var(--red)]/25 bg-[var(--red)]/10 px-4 py-2.5 text-center text-[13px] text-[#ff9a92]"
            >
              {error}
            </motion.p>
          )}
        </AnimatePresence>
      </div>

      <div className="safe-b pt-6">
        <button
          onClick={() => handleJoin()}
          disabled={code.length !== CODE_LENGTH || connecting}
          className="flex w-full items-center justify-center gap-2 rounded-2xl bg-[var(--blue)] py-4 text-[16px] font-semibold text-white transition-all hover:brightness-110 active:scale-[0.985] disabled:pointer-events-none disabled:opacity-30"
        >
          {connecting && <Loader2 className="h-[18px] w-[18px] animate-spin" />}
          {connecting ? 'Connecting…' : 'Join'}
        </button>
      </div>

      <LanguagePicker
        open={pickerOpen}
        selected={language}
        subtitle="What you'll speak and hear"
        onSelect={setLanguage}
        onClose={() => setPickerOpen(false)}
      />
    </motion.div>
  );
}

/** Corner brackets over the camera feed — reads as a viewfinder, not a border. */
function ScannerFrame() {
  const corners = [
    'left-6 top-6 border-l-2 border-t-2 rounded-tl-lg',
    'right-6 top-6 border-r-2 border-t-2 rounded-tr-lg',
    'left-6 bottom-6 border-l-2 border-b-2 rounded-bl-lg',
    'right-6 bottom-6 border-r-2 border-b-2 rounded-br-lg',
  ];
  return (
    <div className="pointer-events-none absolute inset-0">
      {corners.map((position) => (
        <span key={position} className={cn('absolute h-8 w-8 border-white/70', position)} />
      ))}
    </div>
  );
}
