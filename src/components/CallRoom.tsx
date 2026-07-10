import React, { useState, useEffect, useRef, useCallback } from 'react';
import { motion, AnimatePresence } from 'motion/react';
import { Message, Language } from '../types';
import { useSocket } from '../SocketContext';
import { Mic, PhoneOff, MessageSquare, X } from 'lucide-react';
import { cn } from '../utils';

interface CallRoomProps {
  roomCode: string;
  language: Language;
  userName: string;
  onEndCall: () => void;
}

export function CallRoom({ roomCode, language, userName, onEndCall }: CallRoomProps) {
  const { socket } = useSocket();
  const [messages, setMessages] = useState<Message[]>([]);
  const [isListening, setIsListening] = useState(false);
  const [isTranslating, setIsTranslating] = useState(false);
  const [isSpeaking, setIsSpeaking] = useState(false);
  const [interimTranscript, setInterimTranscript] = useState('');
  const [showTranscript, setShowTranscript] = useState(false);
  const [myId] = useState(() => Math.random().toString(36).substring(7));
  
  const recognitionRef = useRef<any>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages, interimTranscript, showTranscript]);

  useEffect(() => {
    if (!socket) return;
    
    // Ensure we are in the room, especially if socket reconnects
    socket.emit('join_room', { roomCode, language: language.name });
    
    const handleConnect = () => {
      socket.emit('join_room', { roomCode, language: language.name });
    };
    socket.on('connect', handleConnect);

    socket.on('new_message', (msg: Message) => {
      setMessages(prev => [...prev, msg]);
      
      if (msg.speakerId !== myId) {
        speakText(msg.translatedText, language.bcp47);
      } else {
        setIsTranslating(false);
      }
    });

    return () => {
      socket.off('connect', handleConnect);
      socket.off('new_message');
    };
  }, [socket, myId, language.name, language.bcp47, roomCode]);

  const speakText = (text: string, langCode: string) => {
    if ('speechSynthesis' in window) {
      window.speechSynthesis.cancel();
      const utterance = new SpeechSynthesisUtterance(text);
      utterance.lang = langCode;
      
      utterance.onstart = () => setIsSpeaking(true);
      utterance.onend = () => setIsSpeaking(false);
      utterance.onerror = (e) => {
        console.error('Speech synthesis error:', e);
        setIsSpeaking(false);
      };
      
      window.speechSynthesis.speak(utterance);
    }
  };

  const startListening = useCallback(() => {
    if (!('webkitSpeechRecognition' in window) && !('SpeechRecognition' in window)) {
      alert('Speech recognition is not supported in this browser. Please use Chrome or Safari.');
      return;
    }

    const SpeechRecognition = (window as any).SpeechRecognition || (window as any).webkitSpeechRecognition;
    const recognition = new SpeechRecognition();
    
    recognition.lang = language.bcp47;
    recognition.continuous = true;
    recognition.interimResults = true;

    recognition.onstart = () => {
      setIsListening(true);
      setInterimTranscript('');
    };

    recognition.onresult = (event: any) => {
      let currentInterim = '';
      for (let i = event.resultIndex; i < event.results.length; ++i) {
        if (!event.results[i].isFinal) {
          currentInterim += event.results[i][0].transcript;
        }
      }
      setInterimTranscript(currentInterim);
    };

    recognition.onerror = (event: any) => {
      console.error('Speech recognition error', event.error);
      stopListening(recognition);
    };

    recognition.start();
    recognitionRef.current = recognition;
  }, [language]);

  const stopListening = useCallback((rec = recognitionRef.current) => {
    if (rec) {
      rec.stop();
      setIsListening(false);
    }
  }, []);

  useEffect(() => {
    if (!recognitionRef.current) return;
    const recognition = recognitionRef.current;
    
    recognition.onresult = (event: any) => {
      let finalTranscript = '';
      let currentInterim = '';
      
      for (let i = event.resultIndex; i < event.results.length; ++i) {
        if (event.results[i].isFinal) {
          finalTranscript += event.results[i][0].transcript;
        } else {
          currentInterim += event.results[i][0].transcript;
        }
      }
      
      setInterimTranscript(currentInterim);
      
      if (finalTranscript.trim() && socket) {
        setIsTranslating(true);
        socket.emit('speech_ended', {
          roomCode,
          originalText: finalTranscript.trim(),
          sourceLang: language.name,
          speakerId: myId,
          speakerName: userName
        });
      }
    };
  }, [socket, myId, roomCode, language.name]);

  const getStateText = () => {
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
            onClick={() => setShowTranscript(true)}
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
            onPointerDown={startListening}
            onPointerUp={() => stopListening()}
            onPointerLeave={() => stopListening()}
            className={cn(
              "relative w-40 h-40 flex items-center justify-center transition-all duration-150 select-none z-10 rounded-full border-4 shadow-[0_0_15px_rgba(0,0,0,0.5)] active:scale-95",
              isListening ? "bg-orange-500 border-orange-400" : 
              isTranslating ? "bg-zinc-800 border-zinc-700" :
              isSpeaking ? "bg-zinc-100 border-zinc-300" :
              "bg-zinc-900 border-zinc-800 hover:bg-zinc-800"
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
            "text-sm font-bold tracking-widest font-mono uppercase",
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
                {messages.length === 0 && !interimTranscript && !isTranslating && (
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
                        <p className={cn("text-[11px] uppercase tracking-widest font-mono", isMe ? "text-orange-500/50" : "text-zinc-600")}>
                          {isMe ? msg.translatedText : msg.originalText}
                        </p>
                      </div>
                    </motion.div>
                  );
                })}
                
                {interimTranscript && (
                  <motion.div
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    className="self-end max-w-[85%] bg-orange-500/5 border border-orange-500/30 text-orange-500 p-4 rounded-none border-r-4"
                  >
                    <p className="text-[15px] animate-pulse opacity-75">{interimTranscript}</p>
                  </motion.div>
                )}

                {isTranslating && (
                   <motion.div
                     initial={{ opacity: 0 }}
                     animate={{ opacity: 1 }}
                     className="self-end max-w-[85%] mt-2"
                   >
                     <div className="flex gap-2 p-3 bg-zinc-900 border border-zinc-800 rounded-none border-r-4 border-r-zinc-600">
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
    </div>
  );
}
