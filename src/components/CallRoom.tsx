import React, { useState, useEffect, useRef, useCallback } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { Message, Language } from '../types';
import { Mic, PhoneOff, MessageSquare, X } from 'lucide-react';
import { cn } from '../utils';

import { connectCall } from '../lib/webrtc';
import { startListening, RecognitionHandle } from '../lib/speechToText';
import { translateText } from '../lib/translate';
import { speak, preloadVoices } from '../lib/textToSpeech';

// Use VITE_ prefix as this is a Vite app. We also check NEXT_PUBLIC_ for fallback if defined.
const SIGNALING_URL = import.meta.env.VITE_SIGNALING_URL || "wss://YOUR-SIGNALING-SERVER-URL";

type CallStatus =
  | "waiting-for-peer"
  | "connecting"
  | "connected"
  | "peer-left"
  | "expired"
  | "error"
  | "disconnected";

interface CallRoomProps {
  roomCode: string;
  language: Language;
  userName: string;
  onEndCall: () => void;
}

export function CallRoom({ roomCode, language, userName, onEndCall }: CallRoomProps) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [isListening, setIsListening] = useState(false);
  const [isTranslating, setIsTranslating] = useState(false);
  const [isSpeaking, setIsSpeaking] = useState(false);
  const [showTranscript, setShowTranscript] = useState(false);
  const [status, setStatus] = useState<CallStatus>("waiting-for-peer");
  const [myId] = useState(() => Math.random().toString(36).substring(7));
  
  const remoteAudioRef = useRef<HTMLAudioElement>(null);
  const callRef = useRef<ReturnType<typeof connectCall> | null>(null);
  const recognitionRef = useRef<RecognitionHandle | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages, showTranscript]);

  // Automatically end the call if the other user leaves
  useEffect(() => {
    if (status === "peer-left") {
      onEndCall();
    }
  }, [status, onEndCall]);

  useEffect(() => {
    preloadVoices();

    const call = connectCall(
      roomCode,
      SIGNALING_URL,
      (stream) => {
        if (remoteAudioRef.current) {
          remoteAudioRef.current.srcObject = stream;
          remoteAudioRef.current.muted = true;
        }
      },
      async (text, lang) => {
        // Receive their original text and language, translate to our language
        setIsTranslating(true);
        try {
          const translated = await translateText(
            text, 
            lang.split("-")[0] || "auto", 
            language.bcp47.split("-")[0]
          );
          setMessages(prev => [...prev, {
            speakerId: "them",
            speakerName: "Stranger",
            originalText: text,
            translatedText: translated
          }]);
          setIsSpeaking(true);
          speak(translated, language.bcp47, () => setIsSpeaking(false));
        } catch (err) {
          console.error("Translation error:", err);
        } finally {
          setIsTranslating(false);
        }
      },
      (newStatus) => setStatus(newStatus as CallStatus)
    );

    callRef.current = call;
    call.addLocalAudio().catch(err => console.error("Mic error:", err));

    return () => {
      recognitionRef.current?.stop();
      call.close();
    };
  }, [roomCode, language.bcp47]);

  const handlePressStart = useCallback(() => {
    if (isSpeaking) return;

    setIsListening(true);
    recognitionRef.current = startListening(
      language.bcp47,
      (recognizedText) => {
        setMessages(prev => [...prev, {
          speakerId: myId,
          speakerName: userName,
          originalText: recognizedText,
          translatedText: recognizedText
        }]);
        
        // Send original text to the peer so they can translate it to their language
        callRef.current?.sendTranslatedText(recognizedText, language.bcp47);
      },
      (err) => {
        console.error("STT error:", err);
        setIsListening(false);
      }
    );
  }, [language.bcp47, myId, userName, isSpeaking]);

  const handlePressEnd = useCallback(() => {
    setIsListening(false);
    recognitionRef.current?.stop();
  }, []);

  const getStateText = () => {
    if (status !== "connected") return `STATUS: ${status.toUpperCase()}`;
    if (isListening) return "RECEIVING SIGNAL...";
    if (isTranslating) return "DECODING...";
    if (isSpeaking) return "TRANSMITTING...";
    return "PTT - PRESS AND HOLD";
  };

  return (
    <div className="flex flex-col min-h-screen bg-[#09090b] text-zinc-100 relative max-w-md mx-auto overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between p-6 z-10 border-b border-zinc-800 bg-zinc-950">
        <div>
          <h2 className="font-mono text-lg font-bold tracking-widest uppercase">{userName || "UNKNOWN USER"}</h2>
          <p className="text-xs font-mono text-zinc-500 uppercase tracking-widest">Freq: {roomCode}</p>
        </div>
        <div className="flex gap-4">
          <button 
            onClick={() => setShowTranscript(v => !v)}
            className="p-3 bg-zinc-900 text-zinc-400 rounded-none border border-zinc-800 hover:bg-zinc-800 transition-colors"
          >
            <MessageSquare className="w-5 h-5" />
          </button>
          <button 
            onClick={onEndCall}
            className="p-3 bg-red-950/30 text-red-500 rounded-none border border-red-900/50 hover:bg-red-900/30 transition-colors"
          >
            <PhoneOff className="w-5 h-5" />
          </button>
        </div>
      </div>

      {/* Main interaction area */}
      <div className="flex-1 flex flex-col items-center justify-center p-8">
        
        <div className="relative flex items-center justify-center w-64 h-64 mb-12">
          {/* Subtle animated background rings */}
          <AnimatePresence>
            {isListening && (
              <motion.div 
                initial={{ opacity: 0, scale: 0.8 }}
                animate={{ opacity: 1, scale: 1.2 }}
                exit={{ opacity: 0, scale: 0.8 }}
                className="absolute inset-0 border border-orange-500/20 rounded-full"
              />
            )}
            {isTranslating && (
              <motion.div 
                initial={{ opacity: 0, scale: 0.8 }}
                animate={{ opacity: 1, scale: 1.2 }}
                exit={{ opacity: 0, scale: 0.8 }}
                className="absolute inset-0 border border-zinc-500/30 rounded-full"
              />
            )}
            {isSpeaking && (
              <motion.div 
                initial={{ opacity: 0, scale: 0.8 }}
                animate={{ opacity: 1, scale: 1.2 }}
                exit={{ opacity: 0, scale: 0.8 }}
                className="absolute inset-0 border border-zinc-100/30 rounded-full"
              />
            )}
          </AnimatePresence>

          <button
            onPointerDown={handlePressStart}
            onPointerUp={handlePressEnd}
            onPointerLeave={handlePressEnd}
            disabled={status !== "connected" && status !== "connecting"}
            className={cn(
              "relative w-40 h-40 flex items-center justify-center transition-all duration-150 select-none z-10 rounded-full border-4 shadow-[0_0_15px_rgba(0,0,0,0.5)]",
              (status !== "connected" && status !== "connecting") ? "opacity-50 cursor-not-allowed bg-zinc-900 border-zinc-800" :
              isListening ? "bg-orange-500 border-orange-400 active:scale-95" : 
              isTranslating ? "bg-zinc-800 border-zinc-700 active:scale-95" :
              isSpeaking ? "bg-zinc-100 border-zinc-300 active:scale-95" :
              "bg-zinc-900 border-zinc-800 hover:bg-zinc-800 active:scale-95"
            )}
          >
            <Mic className={cn(
              "w-14 h-14 transition-colors", 
              isListening ? "text-black" : 
              isTranslating ? "text-zinc-500" :
              isSpeaking ? "text-zinc-900 animate-pulse" :
              "text-zinc-400"
            )} />
            
            {/* Ripple when listening */}
            {isListening && (
              <span className="absolute inset-0 rounded-full border border-orange-500 animate-ping opacity-20"></span>
            )}
          </button>
        </div>

        <motion.p 
          key={getStateText()}
          initial={{ opacity: 0, y: 5 }}
          animate={{ opacity: 1, y: 0 }}
          className={cn(
            "text-sm font-bold tracking-widest font-mono uppercase text-center max-w-[80%]",
            status !== "connected" ? "text-zinc-400" :
            isListening ? "text-orange-500" : 
            isTranslating ? "text-zinc-500" :
            isSpeaking ? "text-zinc-100" :
            "text-zinc-600"
          )}
        >
          {getStateText()}
        </motion.p>
      </div>

      {/* Transcript Overlay */}
      <AnimatePresence>
        {showTranscript && (
          <motion.div 
            initial={{ y: "100%" }}
            animate={{ y: 0 }}
            exit={{ y: "100%" }}
            transition={{ type: "spring", damping: 25, stiffness: 200 }}
            className="absolute inset-x-0 bottom-0 bg-zinc-950 border-t border-zinc-800 shadow-[0_-10px_40px_rgba(0,0,0,0.5)] z-20 flex flex-col"
            style={{ height: '75vh' }}
          >
            <div className="flex items-center justify-between p-6 border-b border-zinc-800 bg-zinc-900">
              <h3 className="font-mono font-bold uppercase tracking-widest text-zinc-100">Live Transcript</h3>
              <button 
                onClick={() => setShowTranscript(false)}
                className="p-2 bg-zinc-800 text-zinc-400 rounded-none hover:bg-zinc-700 transition-colors"
              >
                <X className="w-5 h-5" />
              </button>
            </div>
            
            <div 
              ref={scrollRef}
              className="flex-1 overflow-y-auto p-6 flex flex-col gap-6"
            >
              <AnimatePresence>
                {messages.length === 0 && !isTranslating && (
                  <div className="flex-1 flex items-center justify-center text-zinc-600 text-sm font-mono uppercase tracking-widest">
                    No signals detected
                  </div>
                )}
                
                {messages.map((msg, idx) => {
                  const isMe = msg.speakerId === myId;
                  return (
                    <motion.div
                      key={idx}
                      initial={{ opacity: 0, y: 10 }}
                      animate={{ opacity: 1, y: 0 }}
                      className={cn("flex flex-col max-w-[85%]", isMe ? "self-end items-end" : "self-start items-start")}
                    >
                      <span className="text-[10px] font-mono text-zinc-500 mb-1 tracking-widest uppercase">
                        {isMe ? 'You' : msg.speakerName || 'Stranger'}
                      </span>
                      <div className={cn(
                        "p-4 border",
                        isMe ? "bg-orange-500/10 border-orange-500/50 text-orange-500 rounded-none border-r-4" : "bg-zinc-900 border-zinc-800 text-zinc-300 rounded-none border-l-4"
                      )}>
                        <p className="text-[15px] mb-1 leading-relaxed">{isMe ? msg.originalText : msg.translatedText}</p>
                        {!isMe && (
                          <p className="text-[11px] uppercase tracking-widest font-mono text-zinc-600">
                            {msg.originalText}
                          </p>
                        )}
                      </div>
                    </motion.div>
                  );
                })}

                {isTranslating && (
                   <motion.div
                     initial={{ opacity: 0 }}
                     animate={{ opacity: 1 }}
                     className="self-start max-w-[85%] mt-2"
                   >
                     <div className="flex gap-2 p-3 bg-zinc-900 border border-zinc-800 rounded-none border-l-4 border-l-zinc-600">
                       <div className="w-1.5 h-1.5 rounded-full bg-zinc-500 animate-pulse" style={{ animationDelay: '0ms' }} />
                       <div className="w-1.5 h-1.5 rounded-full bg-zinc-500 animate-pulse" style={{ animationDelay: '150ms' }} />
                       <div className="w-1.5 h-1.5 rounded-full bg-zinc-500 animate-pulse" style={{ animationDelay: '300ms' }} />
                     </div>
                   </motion.div>
                )}
              </AnimatePresence>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
      <audio ref={remoteAudioRef} autoPlay />
    </div>
  );
}
