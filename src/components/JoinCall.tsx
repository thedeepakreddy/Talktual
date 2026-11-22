import { useEffect, useRef, useState } from 'react';
import { Scanner } from '@yudiel/react-qr-scanner';
import { ArrowLeft, Loader2, ScanLine, Keyboard } from 'lucide-react';
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
    <main className="mx-auto flex min-h-dvh w-full max-w-[420px] flex-col px-4 sm:px-6">
      <header className="safe-t flex h-14 items-center gap-1 -ml-2">
        <IconButton label="Back" onClick={onBack}>
          <ArrowLeft className="h-5 w-5" />
        </IconButton>
        <h1 className="text-heading font-semibold">Join a call</h1>
      </header>

      <form
        className="flex flex-1 flex-col"
        onSubmit={(e) => {
          e.preventDefault();
          tryJoin();
        }}
      >
        <div className="flex-1 space-y-5 py-6">
          {invited ? (
            <p className="text-body text-fg-muted">
              You’ve been invited to call <span className="font-mono font-semibold tracking-[0.12em] text-fg">{code}</span>.
            </p>
          ) : scanning ? (
            <div>
              <div className="relative mx-auto aspect-square w-full max-w-[320px] overflow-hidden rounded-lg bg-black">
                {!cameraFailed ? (
                  <Scanner onScan={handleScan} onError={() => setCameraFailed(true)} classNames={{ container: 'w-full h-full object-cover' }} components={{ finder: false }} />
                ) : (
                  <p className="flex h-full items-center justify-center px-8 text-center text-body text-white/80">Camera unavailable.</p>
                )}
              </div>
              <Button variant="ghost" size="sm" className="mx-auto mt-3 flex" icon={<Keyboard className="h-4 w-4" />} onClick={() => setScanning(false)}>
                Enter code
              </Button>
            </div>
          ) : (
            showCodeEntry && (
              <div>
                <label htmlFor="call-code" className="mb-1.5 block text-label font-medium text-fg-muted">
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
                            'flex h-14 flex-1 items-center justify-center rounded-md border bg-surface font-mono text-[1.375rem] font-semibold tnum transition-colors',
                            active ? 'border-focus' : 'border-line'
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
                <Button variant="ghost" size="sm" className="mt-2 -ml-2" icon={<ScanLine className="h-4 w-4" />} onClick={() => setScanning(true)}>
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

        <div className="safe-b">
          <Button type="submit" variant="primary" block disabled={code.length !== CODE_LENGTH || connecting} icon={connecting ? <Loader2 className="h-4 w-4 animate-spin" /> : undefined}>
            {connecting ? 'Connecting…' : 'Join'}
          </Button>
        </div>
      </form>

      <LanguagePicker open={pickerOpen} selected={language} onSelect={(l) => onLanguageChange?.(l)} onClose={() => setPickerOpen(false)} />
    </main>
  );
}
