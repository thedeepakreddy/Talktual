export interface Message {
  originalText: string;
  translatedText: string;
  speakerId: string;
  speakerName?: string;
  sourceLang: string;
  targetLang: string;
}

export interface Language {
  code: string;
  name: string;
  bcp47: string;
}

export const SUPPORTED_LANGUAGES: Language[] = [
  { code: 'en', name: 'English', bcp47: 'en-US' },
  { code: 'es', name: 'Spanish', bcp47: 'es-ES' },
  { code: 'de', name: 'German', bcp47: 'de-DE' },
  { code: 'fa', name: 'Persian', bcp47: 'fa-IR' },
  { code: 'hu', name: 'Hungarian', bcp47: 'hu-HU' },
];
