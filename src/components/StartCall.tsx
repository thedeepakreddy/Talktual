import { useEffect, useState } from 'react';
import { motion } from 'motion/react';
import { QRCodeSVG } from 'qrcode.react';
import { ArrowLeft, Check, Copy, Share2 } from 'lucide-react';
import { Language } from '../types';
import { cn } from '../utils';

interface StartCallProps {
  language: Language;
  onBack: () => void;
  onJoined: (roomCode: string) => void;
}

export function StartCall({ language, onBack, onJoined }: StartCallProps) {
  const [roomCode, setRoomCode] = useState('');
  const [copied, setCopied] = useState<'code' | 'link' | null>(null);

  useEffect(() => {
    const code = Math.random().toString(36).substring(2, 8).toUpperCase();
    setRoomCode(code);

    const wsUrl = import.meta.env.VITE_SIGNALING_URL || 'ws://localhost:8080';
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
      await navigator.share({ title: 'Talktual', text: `Join my channel: ${roomCode}`, url: joinUrl });
    } catch {
      /* dismissed */
    }
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
        <div className="flex items-center gap-2 rounded-full border border-white/8 bg-white/[0.05] px-3 py-1.5">
          <span className="text-[15px] leading-none">{language.flag}</span>
          <span className="text-[13px] font-medium text-[var(--text-2)]">{language.native}</span>
        </div>
      </div>

      <div className="flex flex-1 flex-col items-center justify-center py-4">
        <h1 className="text-[26px] font-bold tracking-[-0.02em] text-[var(--text-1)]">
          Invite someone
        </h1>
        <p className="mt-2 max-w-[18rem] text-center text-[15px] leading-snug text-[var(--text-2)]">
          They scan this, pick their own language, and you’re talking.
        </p>

        <div className="mt-8 rounded-[28px] bg-white p-5 shadow-[0_24px_60px_-20px_rgba(0,0,0,0.9)]">
          {roomCode ? (
            <QRCodeSVG value={joinUrl} size={188} fgColor="#08080a" bgColor="#ffffff" level="M" />
          ) : (
            <div className="h-[188px] w-[188px] animate-pulse rounded-2xl bg-zinc-200" />
          )}
        </div>

        <div className="mt-8 w-full">
          <p className="mb-2.5 text-center text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--text-3)]">
            or share this code
          </p>
          <div className="flex justify-center gap-1.5">
            {(roomCode || '······').split('').map((char, i) => (
              <span
                key={i}
                className="tnum flex h-13 w-11 items-center justify-center rounded-xl border border-white/8 bg-white/[0.06] py-3 font-mono text-[22px] font-semibold text-[var(--text-1)]"
              >
                {char}
              </span>
            ))}
          </div>
        </div>
      </div>

      <div className="safe-b space-y-3">
        <div className="flex gap-3">
          <button
            onClick={() => copy('code')}
            className="flex flex-1 items-center justify-center gap-2 rounded-2xl border border-white/12 bg-white/[0.05] py-3.5 text-[15px] font-semibold text-[var(--text-1)] transition-all hover:bg-white/[0.09] active:scale-[0.985]"
          >
            {copied === 'code' ? (
              <Check className="h-[17px] w-[17px] text-[var(--green)]" />
            ) : (
              <Copy className="h-[17px] w-[17px]" />
            )}
            {copied === 'code' ? 'Copied' : 'Copy code'}
          </button>
          <button
            onClick={share}
            className="flex flex-1 items-center justify-center gap-2 rounded-2xl border border-white/12 bg-white/[0.05] py-3.5 text-[15px] font-semibold text-[var(--text-1)] transition-all hover:bg-white/[0.09] active:scale-[0.985]"
          >
            {copied === 'link' ? (
              <Check className="h-[17px] w-[17px] text-[var(--green)]" />
            ) : (
              <Share2 className="h-[17px] w-[17px]" />
            )}
            {copied === 'link' ? 'Copied' : 'Share link'}
          </button>
        </div>

        <div className="flex items-center justify-center gap-2.5 pt-1">
          <span
            className={cn('led led-pulse')}
            style={{ background: 'var(--amber)' }}
            aria-hidden="true"
          />
          <p className="text-[13px] font-medium text-[var(--text-2)]">
            Waiting for someone to join…
          </p>
        </div>
      </div>
    </motion.div>
  );
}
