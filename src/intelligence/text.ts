// Text helpers shared by ACI (no external dependencies).
import type { AciLang } from './types';

export function aciLang(tag: string | null | undefined): AciLang {
  const b = (tag ?? '').split('-')[0].toLowerCase();
  return (['en', 'es', 'fr', 'de', 'pt', 'it'] as const).includes(b as never) ? (b as AciLang) : 'other';
}

/** Lowercased, punctuation-free text for pattern matching (keeps ¿ and ? for question cues). */
export function norm(text: string): string {
  return text
    .toLowerCase()
    .replace(/[“”"«»]/g, '')
    .replace(/[^\p{L}\p{N}'?¿:\s,]+/gu, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

export function words(text: string): string[] {
  return text
    .toLowerCase()
    .replace(/[^\p{L}\p{N}' ]+/gu, ' ')
    .split(/\s+/)
    .filter(Boolean);
}

/** Whole-word / whole-phrase containment on normalized text. */
export function hasPhrase(haystack: string, phrase: string): boolean {
  const esc = phrase.toLowerCase().replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return new RegExp(`(^|[^\\p{L}\\p{N}])${esc}(?=$|[^\\p{L}\\p{N}])`, 'u').test(haystack.toLowerCase());
}

/** Sensitive identifiers that must never be stored: phones, emails, card/ID-like digit runs, IBANs. */
const SENSITIVE = [
  /[\w.+-]+@[\w-]+\.[\w.]+/g, // email
  /\b[A-Z]{2}\d{2}(?:\s?[A-Z0-9]{4}){2,7}\b/gi, // IBAN-like
  /(?:\+?\d[\s().-]*){7,}/g, // phone / card / long id (7+ digits with separators)
];
export function redactSensitive(text: string): { text: string; found: number } {
  let found = 0;
  let out = text;
  for (const re of SENSITIVE) {
    out = out.replace(re, (m) => {
      // keep short times like "10:30" or years; the phone rule needs 7+ digits anyway
      found++;
      return m.endsWith(' ') ? '[number] ' : '[number]';
    });
  }
  return { text: out, found };
}

export const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
