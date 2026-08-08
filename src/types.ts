export interface Message {
  id: string;
  originalText: string;
  translatedText: string;
  speakerId: string;
  speakerName?: string;
  sourceLang: string;
  targetLang: string;
  /** Milliseconds spent translating this segment, for the latency readout. */
  latencyMs?: number;
}

export interface Language {
  /** ISO 639-1, used for translation. */
  code: string;
  /** English name, for the secondary line in the picker. */
  name: string;
  /** Endonym — what speakers call it themselves. */
  native: string;
  /** Full BCP-47 tag, used for speech recognition and synthesis. */
  bcp47: string;
  flag: string;
}

export const SUPPORTED_LANGUAGES: Language[] = [
  { code: 'en', name: 'English', native: 'English', bcp47: 'en-US', flag: '🇺🇸' },
  { code: 'es', name: 'Spanish', native: 'Español', bcp47: 'es-ES', flag: '🇪🇸' },
  { code: 'fr', name: 'French', native: 'Français', bcp47: 'fr-FR', flag: '🇫🇷' },
  { code: 'de', name: 'German', native: 'Deutsch', bcp47: 'de-DE', flag: '🇩🇪' },
  { code: 'it', name: 'Italian', native: 'Italiano', bcp47: 'it-IT', flag: '🇮🇹' },
  { code: 'pt', name: 'Portuguese', native: 'Português', bcp47: 'pt-BR', flag: '🇧🇷' },
  { code: 'nl', name: 'Dutch', native: 'Nederlands', bcp47: 'nl-NL', flag: '🇳🇱' },
  { code: 'pl', name: 'Polish', native: 'Polski', bcp47: 'pl-PL', flag: '🇵🇱' },
  { code: 'ru', name: 'Russian', native: 'Русский', bcp47: 'ru-RU', flag: '🇷🇺' },
  { code: 'uk', name: 'Ukrainian', native: 'Українська', bcp47: 'uk-UA', flag: '🇺🇦' },
  { code: 'tr', name: 'Turkish', native: 'Türkçe', bcp47: 'tr-TR', flag: '🇹🇷' },
  { code: 'ar', name: 'Arabic', native: 'العربية', bcp47: 'ar-SA', flag: '🇸🇦' },
  { code: 'fa', name: 'Persian', native: 'فارسی', bcp47: 'fa-IR', flag: '🇮🇷' },
  { code: 'he', name: 'Hebrew', native: 'עברית', bcp47: 'he-IL', flag: '🇮🇱' },
  { code: 'hi', name: 'Hindi', native: 'हिन्दी', bcp47: 'hi-IN', flag: '🇮🇳' },
  { code: 'te', name: 'Telugu', native: 'తెలుగు', bcp47: 'te-IN', flag: '🇮🇳' },
  { code: 'ta', name: 'Tamil', native: 'தமிழ்', bcp47: 'ta-IN', flag: '🇮🇳' },
  { code: 'ja', name: 'Japanese', native: '日本語', bcp47: 'ja-JP', flag: '🇯🇵' },
  { code: 'ko', name: 'Korean', native: '한국어', bcp47: 'ko-KR', flag: '🇰🇷' },
  { code: 'zh', name: 'Chinese', native: '中文', bcp47: 'zh-CN', flag: '🇨🇳' },
  { code: 'id', name: 'Indonesian', native: 'Bahasa Indonesia', bcp47: 'id-ID', flag: '🇮🇩' },
  { code: 'vi', name: 'Vietnamese', native: 'Tiếng Việt', bcp47: 'vi-VN', flag: '🇻🇳' },
  { code: 'th', name: 'Thai', native: 'ไทย', bcp47: 'th-TH', flag: '🇹🇭' },
  { code: 'sv', name: 'Swedish', native: 'Svenska', bcp47: 'sv-SE', flag: '🇸🇪' },
  { code: 'da', name: 'Danish', native: 'Dansk', bcp47: 'da-DK', flag: '🇩🇰' },
  { code: 'no', name: 'Norwegian', native: 'Norsk', bcp47: 'nb-NO', flag: '🇳🇴' },
  { code: 'fi', name: 'Finnish', native: 'Suomi', bcp47: 'fi-FI', flag: '🇫🇮' },
  { code: 'cs', name: 'Czech', native: 'Čeština', bcp47: 'cs-CZ', flag: '🇨🇿' },
  { code: 'el', name: 'Greek', native: 'Ελληνικά', bcp47: 'el-GR', flag: '🇬🇷' },
  { code: 'ro', name: 'Romanian', native: 'Română', bcp47: 'ro-RO', flag: '🇷🇴' },
  { code: 'hu', name: 'Hungarian', native: 'Magyar', bcp47: 'hu-HU', flag: '🇭🇺' },
];

export function findLanguage(code: string | undefined | null): Language | undefined {
  if (!code) return undefined;
  const base = code.split('-')[0].toLowerCase();
  return SUPPORTED_LANGUAGES.find((l) => l.code === base || l.bcp47.toLowerCase() === code.toLowerCase());
}

/** Best guess at the user's language from their browser, defaulting to English. */
export function detectLanguage(): Language {
  for (const tag of navigator.languages ?? [navigator.language]) {
    const match = findLanguage(tag);
    if (match) return match;
  }
  return SUPPORTED_LANGUAGES[0];
}
