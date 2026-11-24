import { useEffect, useRef, useState } from 'react';
import { Scanner } from '@yudiel/react-qr-scanner';
import { ArrowLeft, ArrowRight, Loader2, ScanLine, Keyboard } from './icons';
import { Language } from '../types';
import { LanguagePicker } from './LanguagePicker';
import { LanguageField, NameField } from './LanguageField';
import { Button, IconButton, Banner } from './ui';
import { cn } from '../utils';
import { SIGNALING_URL } from '../lib/config';
import { CODE_LENGTH, codeFromScan, normalizeRoomCode } from '../lib/roomCode';

interface JoinCallProps {
  language: Language;
  onLanguageChange?: (language: Language) => void;
  userName?: string;
  onUserNameChange?: (name: string) => void;
  onBack: () => void;
  onJoined: (roomCode: string) => void;
  initialCode?: string;
  /** Opened from an invite link: the code is known; ask only for name and language. */
  invited?: boolean;
}

/**
 * How long to wait for the host before giving up. The host is already sitting
 * in the lobby, so a valid code pairs almost instantly; silence means the code
 * is wrong or the host left.
 */
export const JOIN_TIMEOUT_MS = 12_000;

export function JoinCall({
  language,
  onLanguageChange,
  userName = '',
  onUserNameChange,
  onBack,
  onJoined,
  initialCode = '',
  invited = false,
}: JoinCallProps) {
  const [code, setCode] = useState(() => normalizeRoomCode(initialCode));
  const [scanning, setScanning] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [nameError, setNameError] = useState<string | null>(null);
  const [cameraFailed, setCameraFailed] = useState(false);
  const [localName, setLocalName] = useState(userName);
  const name = onUserNameChange ? userName : localName;
  const setName = (v: string) => (onUserNameChange ? onUserNameChange(v) : setLocalName(v));

  const isConnectingRef = useRef(false);
  const codeInputRef = useRef<HTMLInputElement>(null);
  const nameRef = useRef<HTMLInputElement>(null);
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

  /** Name is needed only when arriving from a link (home already asked for it). */
  const needsName = invited;
  const tryJoin = (joinCode: string = code) => {
    if (needsName && !name.trim()) {
      setNameError('Add your name so the other person knows who’s talking.');
      nameRef.current?.focus();
      return;
    }
    handleJoin(joinCode);
  };

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
      () => fail('No one is waiting with that code. Check it and try again.'),
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
        if (mountedRef.current) onJoined(codeUpper);
      } else if (msg?.type === 'error') {
        fail(
          msg.code === 'session_full'
            ? 'That call already has two people.'
            : 'That code isn’t valid. Check it and try again.'
        );
      }
    };

    ws.onclose = () => fail('Lost connection to the server. Try again.');
  };

  const handleScan = (detectedCodes: { rawValue: string }[]) => {
    if (!detectedCodes?.length) return;
    const scanned = codeFromScan(detectedCodes[0].rawValue);
    if (scanned) {
      setCode(scanned);
      setScanning(false);
      tryJoin(scanned);
    }
  };

  const showCodeEntry = !invited;

  return (
    <main className="mx-auto flex min-h-dvh w-full max-w-[460px] flex-col px-4 sm:px-6 lg:justify-center lg:py-10">
      <header className="safe-t flex h-[68px] items-center gap-3">
        <IconButton label="Back" onClick={onBack}>
          <ArrowLeft className="h-5 w-5" />
        </IconButton>
        <h1 className="t-title text-heading">Join a call</h1>
      </header>

      <form
        className="flex flex-1 flex-col lg:flex-none"
        onSubmit={(e) => {
          e.preventDefault();
          tryJoin();
        }}
      >
        <div className="glass-strong my-4 space-y-6 rounded-[32px] p-5 sm:p-6">
          {invited ? (
            <p className="text-body-lg text-fg-muted">
              You’ve been invited to call <span className="font-mono font-semibold tracking-[0.12em] text-fg">{code}</span>.
            </p>
          ) : scanning ? (
            <div>
              <div className="relative mx-auto aspect-square w-full max-w-[320px] overflow-hidden rounded-[28px] bg-black shadow-[inset_0_0_0_1px_var(--glass-stroke)]">
                {!cameraFailed ? (
                  <Scanner onScan={handleScan} onError={() => setCameraFailed(true)} classNames={{ container: 'w-full h-full object-cover' }} components={{ finder: false }} />
                ) : (
                  <p className="flex h-full items-center justify-center px-8 text-center text-body text-white/80">Camera unavailable.</p>
                )}
              </div>
              <Button variant="tertiary" size="sm" className="mx-auto mt-3 flex" icon={<Keyboard className="h-4 w-4" />} onClick={() => setScanning(false)}>
                Enter code
              </Button>
            </div>
          ) : (
            showCodeEntry && (
              <div>
                <label htmlFor="call-code" className="t-eyebrow mb-2 block px-1 text-fg-subtle">
                  Call code
                </label>
                <div className="relative">
                  <div className="flex gap-2" aria-hidden="true">
                    {Array.from({ length: CODE_LENGTH }).map((_, i) => {
                      const char = code[i];
                      const active = i === code.length;
                      return (
                        <span
                          key={i}
                          className={cn(
                            'glass-clear flex h-[60px] min-w-0 flex-1 items-center justify-center rounded-[18px] font-mono text-[1.5rem] font-semibold tnum transition-shadow',
                            active && 'shadow-[inset_0_0_0_1.5px_var(--focus),0_0_0_4px_color-mix(in_oklab,var(--accent)_16%,transparent)]'
                          )}
                        >
                          {char ?? ''}
                        </span>
                      );
                    })}
                  </div>
                  <input
                    id="call-code"
                    ref={codeInputRef}
                    value={code}
                    autoFocus
                    onChange={(e) => {
                      const next = normalizeRoomCode(e.target.value);
                      setCode(next);
                      setError(null);
                      if (next.length === CODE_LENGTH) tryJoin(next);
                    }}
                    inputMode="text"
                    autoComplete="one-time-code"
                    autoCapitalize="characters"
                    autoCorrect="off"
                    spellCheck={false}
                    maxLength={CODE_LENGTH}
                    aria-label="Call code"
                    className="absolute inset-0 h-full w-full cursor-text opacity-0"
                  />
                </div>
                <Button variant="tertiary" size="sm" className="mt-3" icon={<ScanLine className="h-4 w-4" />} onClick={() => setScanning(true)}>
                  Scan a QR code instead
                </Button>
              </div>
            )
          )}

          {needsName && (
            <NameField
              inputRef={nameRef}
              value={name}
              error={nameError}
              onChange={(v) => {
                setName(v);
                if (v.trim()) setNameError(null);
              }}
            />
          )}
          <LanguageField label="You speak and hear" language={language} onOpen={() => setPickerOpen(true)} />

          {error && <Banner tone="error">{error}</Banner>}
        </div>

        <div className="safe-b mt-auto lg:mt-2">
          <Button type="submit" variant="primary" size="lg" block disabled={code.length !== CODE_LENGTH || connecting} icon={connecting ? <Loader2 className="h-4 w-4 animate-spin" /> : undefined} chip={connecting ? undefined : <ArrowRight />}>
            {connecting ? 'Connecting…' : 'Join'}
          </Button>
        </div>
      </form>

      <LanguagePicker open={pickerOpen} selected={language} onSelect={(l) => onLanguageChange?.(l)} onClose={() => setPickerOpen(false)} />
    </main>
  );
}
