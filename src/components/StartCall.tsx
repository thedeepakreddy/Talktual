import React, { useEffect, useState } from 'react';
import { motion } from 'motion/react';
import { QRCodeSVG } from 'qrcode.react';
import { Language } from '../types';
import { ArrowLeft, Loader2 } from 'lucide-react';

interface StartCallProps {
  language: Language;
  onBack: () => void;
  onJoined: (roomCode: string) => void;
}

export function StartCall({ language, onBack, onJoined }: StartCallProps) {
  const [roomCode, setRoomCode] = useState('');

  useEffect(() => {
    // Generate random 6-character code
    const code = Math.random().toString(36).substring(2, 8).toUpperCase();
    setRoomCode(code);

    const wsUrl = import.meta.env.VITE_SIGNALING_URL || "ws://localhost:8080";
    let ws: WebSocket;
    let isConnecting = false;
    let isMounted = true;

    const connect = () => {
      if (!isMounted || isConnecting) return;
      isConnecting = true;
      ws = new WebSocket(wsUrl);

      ws.onopen = () => {
        isConnecting = false;
        ws.send(JSON.stringify({ type: "join", sessionId: code + "_signal" }));
      };

      ws.onerror = () => {
        isConnecting = false;
        console.error("WebSocket connection failed to", wsUrl);
      };

      ws.onmessage = (event) => {
        try {
          const msg = JSON.parse(event.data);
          if (msg.type === "ready") {
            isMounted = false; // Prevent reconnect
            ws.close();
            onJoined(code);
          }
        } catch(e) {}
      };

      ws.onclose = () => {
        isConnecting = false;
        // Reconnect if it dropped unexpectedly
        if (isMounted) {
          setTimeout(connect, 2000);
        }
      };
    };

    connect();

    return () => {
      isMounted = false;
      if (ws) ws.close();
    };
  }, [onJoined]);

  // Construct a join URL
  const joinUrl = typeof window !== 'undefined' ? `${window.location.origin}/?join=${roomCode}` : '';

  return (
    <motion.div 
      initial={{ opacity: 0, x: 20 }}
      animate={{ opacity: 1, x: 0 }}
      className="flex flex-col items-center min-h-screen p-8 max-w-md mx-auto relative bg-[#09090b] text-zinc-100"
    >
      <button 
        onClick={onBack}
        className="absolute top-8 left-8 p-3 rounded-none bg-zinc-900 border border-zinc-800 hover:bg-zinc-800 transition-colors"
      >
        <ArrowLeft className="w-5 h-5 text-zinc-100" />
      </button>

      <div className="text-center mt-20 mb-16">
        <h2 className="text-xl font-bold mb-3 font-mono tracking-widest uppercase">Scan to Sync</h2>
        <p className="text-zinc-500 font-mono text-xs uppercase tracking-widest">Base Freq: {language.name}</p>
      </div>

      <div className="bg-white p-6 rounded-none border-4 border-zinc-800 flex flex-col items-center justify-center mb-10">
        {roomCode ? (
          <QRCodeSVG value={joinUrl} size={180} fgColor="#09090b" bgColor="#ffffff" />
        ) : (
          <div className="w-[180px] h-[180px] flex items-center justify-center bg-zinc-100">
            <Loader2 className="w-8 h-8 animate-spin text-zinc-900" />
          </div>
        )}
      </div>

      <div className="text-center mb-20">
        <p className="text-xs font-mono text-zinc-500 mb-4 uppercase tracking-widest">Manual Override Code</p>
        <p className="text-4xl font-mono font-bold tracking-[0.2em] text-orange-500">{roomCode || '------'}</p>
      </div>

      <div className="flex items-center justify-center gap-3 text-zinc-400 mt-auto mb-8 font-mono text-xs uppercase tracking-widest">
        <span className="relative flex h-2 w-2">
          <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-orange-400 opacity-75"></span>
          <span className="relative inline-flex rounded-full h-2 w-2 bg-orange-500"></span>
        </span>
        <p>Awaiting Signal...</p>
      </div>
    </motion.div>
  );
}
