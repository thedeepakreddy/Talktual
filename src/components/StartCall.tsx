import { useEffect, useState } from 'react';
import { QRCodeSVG } from 'qrcode.react';
import { ArrowLeft, Check, Copy, Share } from './icons';
import { Language } from '../types';
import { SIGNALING_URL } from '../lib/config';
import { generateRoomCode } from '../lib/roomCode';
import { Button, IconButton } from './ui';

interface StartCallProps {
  language: Language;
  onBack: () => void;
  onJoined: (roomCode: string) => void;
}

export function StartCall({ language, onBack, onJoined }: StartCallProps) {
  const [roomCode, setRoomCode] = useState('');
  const [copied, setCopied] = useState<'code' | 'link' | null>(null);

  useEffect(() => {
    const code = generateRoomCode();
    setRoomCode(code);

    const wsUrl = SIGNALING_URL;
    let ws: WebSocket;
    let isConnecting = false;
    let isMounted = true;

    const connect = () => {
      if (!isMounted || isConnecting) return;
      isConnecting = true;
      ws = new WebSocket(wsUrl);

      ws.onopen = () => {
        isConnecting = false;
        ws.send(JSON.stringify({ type: 'join', sessionId: code + '_signal' }));
      };

      ws.onerror = () => {
        isConnecting = false;
        console.error('WebSocket connection failed to', wsUrl);
      };

      ws.onmessage = (event) => {
        try {
          const msg = JSON.parse(event.data);
          if (msg.type === 'ready') {
            isMounted = false;
            ws.close();
            onJoined(code);
          }
        } catch {
          /* ignore malformed frames */
        }
      };

      ws.onclose = () => {
        isConnecting = false;
        if (isMounted) setTimeout(connect, 2000);
      };
    };

    connect();

    return () => {
      isMounted = false;
      if (ws) ws.close();
    };
  }, [onJoined]);

  const joinUrl = typeof window !== 'undefined' ? `${window.location.origin}/?join=${roomCode}` : '';

  const copy = async (what: 'code' | 'link') => {
    try {
      await navigator.clipboard.writeText(what === 'code' ? roomCode : joinUrl);
      setCopied(what);
      setTimeout(() => setCopied(null), 1800);
    } catch {
      /* clipboard unavailable */
    }
  };

  const share = async () => {
    if (!navigator.share) return copy('link');
    try {
      await navigator.share({ title: 'Talktual', text: `Join my Talktual call: ${roomCode}`, url: joinUrl });
    } catch {
      /* dismissed */
    }
  };

  const canShare = typeof navigator !== 'undefined' && typeof navigator.share === 'function';

  return (
    <main className="mx-auto flex min-h-dvh w-full max-w-[460px] flex-col px-4 sm:px-6 lg:justify-center lg:py-10">
      <header className="safe-t flex h-[68px] items-center gap-3">
        <IconButton label="Back" onClick={onBack}>
          <ArrowLeft className="h-5 w-5" />
        </IconButton>
        <h1 className="t-title text-heading">Invite someone</h1>
      </header>

      <div className="flex flex-1 flex-col items-center justify-center py-6 text-center lg:flex-none">
        <div className="glass-strong w-full rounded-[36px] px-6 pb-7 pt-6">
          <p className="text-body-lg text-fg-muted">They scan this, or open your link.</p>
          {/* QR codes need a light, high-contrast field to scan reliably in any theme. */}
          <div className="mx-auto mt-6 w-fit rounded-[28px] bg-white p-4 shadow-[0_20px_50px_-16px_var(--glow),inset_0_0_0_1px_rgb(0_0_0/0.04)]">
            {roomCode ? (
              <QRCodeSVG value={joinUrl} size={196} fgColor="#140a04" bgColor="#ffffff" level="M" title="Invite QR code" />
            ) : (
              <div className="h-[196px] w-[196px]" aria-hidden="true" />
            )}
          </div>
          <p className="t-eyebrow mt-7 text-fg-subtle">Call code</p>
          <p data-call-code className="text-ember mt-2 font-mono text-[2.125rem] font-semibold tracking-[0.2em] tnum" aria-label={roomCode.split('').join(' ')}>
            {roomCode || '······'}
          </p>
          <p role="status" className="glass-clear mx-auto mt-6 flex w-fit items-center gap-2 rounded-full px-4 py-2 text-label text-fg-muted">
            <span className="h-2 w-2 rounded-full bg-accent shadow-[0_0_10px_var(--accent)]" aria-hidden="true" />
            Waiting for them to join…
          </p>
        </div>
      </div>

      <div className="safe-b grid grid-cols-2 gap-3">
        <Button variant="secondary" size="lg" className="px-4" onClick={() => copy('code')} icon={copied === 'code' ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}>
          {copied === 'code' ? 'Copied' : 'Copy code'}
        </Button>
        <Button variant="primary" size="lg" className="px-4" onClick={share} icon={copied === 'link' ? <Check className="h-4 w-4" /> : <Share className="h-4 w-4" />}>
          {copied === 'link' ? 'Link copied' : canShare ? 'Share link' : 'Copy link'}
        </Button>
      </div>
    </main>
  );
}
