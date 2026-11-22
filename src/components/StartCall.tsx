import { useEffect, useState } from 'react';
import { QRCodeSVG } from 'qrcode.react';
import { ArrowLeft, Check, Copy, Share } from 'lucide-react';
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
    <main className="mx-auto flex min-h-dvh w-full max-w-[420px] flex-col px-4 sm:px-6">
      <header className="safe-t flex h-14 items-center gap-1 -ml-2">
        <IconButton label="Back" onClick={onBack}>
          <ArrowLeft className="h-5 w-5" />
        </IconButton>
        <h1 className="text-heading font-semibold">Invite someone</h1>
      </header>

      <div className="flex flex-1 flex-col items-center justify-center py-6 text-center">
        <p className="text-body text-fg-muted">They scan this, or open your link.</p>
        {/* QR codes need a light, high-contrast field to scan reliably in any theme. */}
        <div className="mt-6 rounded-lg bg-white p-4">
          {roomCode ? (
            <QRCodeSVG value={joinUrl} size={192} fgColor="#0e0f11" bgColor="#ffffff" level="M" title="Invite QR code" />
          ) : (
            <div className="h-[192px] w-[192px]" aria-hidden="true" />
          )}
        </div>
        <p className="mt-6 text-label text-fg-muted">Call code</p>
        <p data-call-code className="mt-1 font-mono text-[1.75rem] font-semibold tracking-[0.18em] tnum" aria-label={roomCode.split('').join(' ')}>
          {roomCode || '······'}
        </p>
        <p role="status" className="mt-6 flex items-center gap-2 text-label text-fg-muted">
          <span className="pulse-dot h-2 w-2 rounded-full bg-accent" aria-hidden="true" />
          Waiting for them to join…
        </p>
      </div>

      <div className="safe-b grid grid-cols-2 gap-3">
        <Button variant="secondary" onClick={() => copy('code')} icon={copied === 'code' ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}>
          {copied === 'code' ? 'Copied' : 'Copy code'}
        </Button>
        <Button variant="primary" onClick={share} icon={copied === 'link' ? <Check className="h-4 w-4" /> : <Share className="h-4 w-4" />}>
          {copied === 'link' ? 'Link copied' : canShare ? 'Share link' : 'Copy link'}
        </Button>
      </div>
    </main>
  );
}
