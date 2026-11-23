// Display/delivery preferences (never conversation content). Local only.

export interface VoicePrefs {
  /** Speak AIR translations aloud by default (off = text only). */
  speak: boolean;
  /** Spoken translation level 0..1. */
  volume: number;
}

const KEY = 'talktual.voice.v1';
export const DEFAULT_VOICE: VoicePrefs = { speak: true, volume: 0.7 };

export function getVoicePrefs(): VoicePrefs {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? 'null');
    if (v && typeof v.speak === 'boolean' && typeof v.volume === 'number') return { speak: v.speak, volume: Math.min(1, Math.max(0, v.volume)) };
  } catch {
    /* default */
  }
  return { ...DEFAULT_VOICE };
}

export function setVoicePrefs(p: VoicePrefs): void {
  try {
    localStorage.setItem(KEY, JSON.stringify({ speak: p.speak, volume: p.volume }));
  } catch {
    /* page only */
  }
}

const PROCESSING_KEY = 'talktual.processing.mode.v1';
export type ModePref = 'cloud' | 'hybrid' | 'local-only';
export function getModePref(): ModePref {
  try {
    const v = localStorage.getItem(PROCESSING_KEY);
    return v === 'hybrid' || v === 'local-only' ? v : 'cloud';
  } catch {
    return 'cloud';
  }
}
export function setModePref(m: ModePref): void {
  try {
    localStorage.setItem(PROCESSING_KEY, m);
  } catch {
    /* page only */
  }
}
export const MODE_LABEL: Record<ModePref, string> = { cloud: 'Cloud', hybrid: 'Hybrid', 'local-only': 'On-device only' };
