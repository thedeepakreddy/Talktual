import React from 'react';
import { motion } from 'motion/react';
import { SUPPORTED_LANGUAGES, Language } from '../types';

interface HomeProps {
  onStartCall: (lang: Language, name: string) => void;
  onJoinCall: (lang: Language, name: string) => void;
}

export function Home({ onStartCall, onJoinCall }: HomeProps) {
  const [selectedLanguage, setSelectedLanguage] = React.useState<Language>(SUPPORTED_LANGUAGES[0]);
  const [userName, setUserName] = React.useState('');

  return (
    <motion.div 
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      className="flex flex-col items-center justify-center min-h-screen p-8 max-w-md mx-auto"
    >
      <div className="text-center mb-16">
        <h1 className="text-4xl font-bold tracking-tight mb-3 text-white uppercase">Talktual</h1>
        <p className="text-zinc-500 font-mono text-sm tracking-widest uppercase">
          by <a href="https://askdeepakai-datascientist.onrender.com/" target="_blank" rel="noopener noreferrer" className="hover:text-orange-500 transition-colors">AskDeepakAI</a>
        </p>
      </div>

      <div className="w-full mb-6">
        <label className="block text-xs font-mono tracking-widest text-zinc-500 mb-3 uppercase">Operative Name</label>
        <input
          type="text"
          placeholder="ENTER NAME"
          value={userName}
          onChange={(e) => setUserName(e.target.value)}
          className="w-full p-4 bg-zinc-900/50 text-white border border-zinc-800 rounded-none focus:outline-none focus:border-orange-500 transition-colors font-mono uppercase text-sm placeholder:text-zinc-700"
        />
      </div>

      <div className="w-full mb-10">
        <label className="block text-xs font-mono tracking-widest text-zinc-500 mb-3 uppercase">Your Language Frequency</label>
        <select 
          className="w-full p-4 bg-zinc-900/50 text-white border border-zinc-800 rounded-none focus:outline-none focus:border-orange-500 transition-colors font-mono uppercase text-sm"
          value={selectedLanguage.code}
          onChange={(e) => {
            const lang = SUPPORTED_LANGUAGES.find(l => l.code === e.target.value);
            if (lang) setSelectedLanguage(lang);
          }}
        >
          {SUPPORTED_LANGUAGES.map(lang => (
            <option key={lang.code} value={lang.code}>{lang.name}</option>
          ))}
        </select>
      </div>

      <div className="flex flex-col gap-4 w-full mt-4">
        <button
          onClick={() => onStartCall(selectedLanguage, userName)}
          disabled={!userName.trim()}
          className="w-full py-4 px-6 bg-orange-500 text-black font-bold uppercase tracking-widest text-sm transition-colors hover:bg-orange-400 rounded-none disabled:opacity-50 disabled:cursor-not-allowed"
        >
          Start a call
        </button>
        <button
          onClick={() => onJoinCall(selectedLanguage, userName)}
          disabled={!userName.trim()}
          className="w-full py-4 px-6 bg-transparent text-white border border-zinc-700 font-bold uppercase tracking-widest text-sm transition-colors hover:bg-zinc-800 rounded-none disabled:opacity-50 disabled:cursor-not-allowed"
        >
          Join Signal
        </button>
      </div>
    </motion.div>
  );
}
