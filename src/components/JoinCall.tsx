import React, { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { Language, SUPPORTED_LANGUAGES } from '../types';
import { ArrowLeft, Camera, Keyboard } from 'lucide-react';
import { Scanner } from '@yudiel/react-qr-scanner';

interface JoinCallProps {
  language: Language;
  onBack: () => void;
  onJoined: (roomCode: string, lang: Language) => void;
  initialCode?: string;
}

type JoinMode = 'scan' | 'manual';

export function JoinCall({ language: initialLanguage, onBack, onJoined, initialCode = '' }: JoinCallProps) {
  const [code, setCode] = useState(initialCode);
  const [mode, setMode] = useState<JoinMode>('scan');
  const [language, setLanguage] = useState(initialLanguage);

  useEffect(() => {
    if (initialCode && initialCode.length === 6) {
      setMode('manual');
    }
  }, [initialCode]);

  const handleJoin = (joinCode: string = code) => {
    if (joinCode.length === 6) {
      const codeUpper = joinCode.toUpperCase();
      const wsUrl = import.meta.env.VITE_SIGNALING_URL || "ws://localhost:8080";
      const ws = new WebSocket(wsUrl);

      ws.onopen = () => {
        // Send a join to the special signaling room to trigger the QR screen to advance
        ws.send(JSON.stringify({ type: "join", sessionId: codeUpper + "_signal" }));
      };

      ws.onmessage = (event) => {
        try {
          const msg = JSON.parse(event.data);
          if (msg.type === "ready") {
            ws.close();
            onJoined(codeUpper, language);
          }
        } catch(e) {}
      };

      // Fallback in case the other peer already dropped their watcher or network is slow
      setTimeout(() => {
        if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) {
          ws.close();
          onJoined(codeUpper, language);
        }
      }, 1500);
    }
  };

  const handleScan = (detectedCodes: any[]) => {
    if (detectedCodes && detectedCodes.length > 0) {
      const result = detectedCodes[0].rawValue;
      // Extract code from URL if it's a join URL
      const urlMatch = result.match(/\?join=([a-zA-Z0-9]{6})/i);
      if (urlMatch && urlMatch[1]) {
        handleJoin(urlMatch[1].toUpperCase());
      } else if (result.length === 6) {
        handleJoin(result.toUpperCase());
      }
    }
  };

  return (
    <motion.div 
      initial={{ opacity: 0, x: 20 }}
      animate={{ opacity: 1, x: 0 }}
      className="flex flex-col min-h-screen p-8 max-w-md mx-auto relative bg-[#09090b] text-zinc-100"
    >
      <button 
        onClick={onBack}
        className="absolute top-8 left-8 p-3 rounded-none bg-zinc-900 border border-zinc-800 hover:bg-zinc-800 transition-colors z-10"
      >
        <ArrowLeft className="w-5 h-5 text-zinc-100" />
      </button>

      <div className="text-center mt-24 mb-10">
        <h2 className="text-xl font-bold mb-3 font-mono tracking-widest uppercase">Sync Signal</h2>
        
        <div className="mt-4 flex flex-col items-center">
          <label className="text-xs text-zinc-500 mb-1 uppercase tracking-widest font-mono">Your Freq</label>
          <select 
            className="p-2 bg-transparent text-zinc-300 font-mono text-sm tracking-widest text-center focus:outline-none cursor-pointer uppercase"
            value={language.code}
            onChange={(e) => {
              const lang = SUPPORTED_LANGUAGES.find(l => l.code === e.target.value);
              if (lang) setLanguage(lang);
            }}
          >
            {SUPPORTED_LANGUAGES.map(lang => (
              <option key={lang.code} value={lang.code} className="bg-zinc-900 text-zinc-100">{lang.name}</option>
            ))}
          </select>
        </div>
      </div>

      <div className="flex-1 flex flex-col items-center justify-center w-full mb-10">
        <AnimatePresence mode="wait">
          {mode === 'scan' ? (
            <motion.div 
              key="scan"
              initial={{ opacity: 0, scale: 0.95 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.95 }}
              className="w-full max-w-[280px] aspect-square overflow-hidden bg-zinc-900 relative border-2 border-zinc-800"
            >
              <Scanner 
                onScan={handleScan}
                classNames={{
                  container: "w-full h-full object-cover",
                }}
                components={{
                  audio: false,
                  finder: false,
                }}
              />
              <div className="absolute inset-0 border-[40px] border-black/50 pointer-events-none" />
              <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
                <div className="w-3/4 h-3/4 border border-orange-500/50" />
                {/* Crosshairs */}
                <div className="absolute w-full h-[1px] bg-orange-500/20" />
                <div className="absolute h-full w-[1px] bg-orange-500/20" />
              </div>
            </motion.div>
          ) : (
            <motion.div 
              key="manual"
              initial={{ opacity: 0, scale: 0.95 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.95 }}
              className="w-full flex flex-col items-center"
            >
              <input
                type="text"
                maxLength={6}
                value={code}
                onChange={(e) => setCode(e.target.value.toUpperCase())}
                placeholder="------"
                className="w-full max-w-[280px] text-center text-4xl font-mono font-bold tracking-[0.2em] p-6 bg-zinc-900/50 border-b-2 border-zinc-800 focus:outline-none focus:border-orange-500 transition-colors uppercase placeholder-zinc-700 rounded-none mb-8 text-orange-500"
              />
              
              <button
                onClick={() => handleJoin()}
                disabled={code.length !== 6}
                className="w-full max-w-[280px] py-4 px-6 bg-orange-500 text-black font-bold uppercase tracking-widest text-sm transition-colors hover:bg-orange-400 disabled:opacity-50 disabled:cursor-not-allowed rounded-none"
              >
                Establish Link
              </button>
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      <div className="mt-auto mb-8 flex justify-center">
        <button
          onClick={() => setMode(mode === 'scan' ? 'manual' : 'scan')}
          className="flex items-center gap-2 text-xs font-mono uppercase tracking-widest text-zinc-500 hover:text-zinc-300 transition-colors"
        >
          {mode === 'scan' ? (
            <>
              <Keyboard className="w-4 h-4" />
              Manual Override
            </>
          ) : (
            <>
              <Camera className="w-4 h-4" />
              Optical Scan
            </>
          )}
        </button>
      </div>
    </motion.div>
  );
}
