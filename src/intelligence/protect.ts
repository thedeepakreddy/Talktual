// MT-input construction and restoration. The ORIGINAL text is what is shown
// and sent as `original`; only the translator's input may differ, and only by
// (1) placeholder tokens for names kept verbatim and (2) lexicon sense edits.

import { escapeRe } from './text';
import type { Protection, SenseEdit } from './types';

export const token = (i: number) => `QX${i}Q`;
const TOKEN_RE = /Q\s*X\s*(\d+)\s*Q/gi;

const wordRe = (w: string) => new RegExp(`(^|[^\\p{L}\\p{N}])(${escapeRe(w)})(?=$|[^\\p{L}\\p{N}])`, 'iu');

/** Deterministic: same original + same protections/edits → same MT input. */
export function buildMtInput(original: string, protections: Protection[], edits: SenseEdit[]): string {
  let out = original;
  protections.forEach((p) => {
    out = out.replace(new RegExp(`(^|[^\\p{L}\\p{N}])${escapeRe(p.value)}(?=$|[^\\p{L}\\p{N}])`, 'gu'), (_m, pre) => `${pre}${p.token}`);
  });
  for (const e of edits) {
    out = out.replace(wordRe(e.word), (_m, pre, w: string) => `${pre}${/^\p{Lu}/u.test(w) ? e.replacement[0].toUpperCase() + e.replacement.slice(1) : e.replacement}`);
  }
  return out;
}

/** Restores tokens. ok=false if any token is missing, duplicated or unknown (malformed provider output). */
export function restore(translated: string, protections: Protection[]): { text: string; ok: boolean } {
  if (!protections.length) return { text: translated, ok: !TOKEN_RE.test(translated) };
  const counts = new Map<number, number>();
  for (const m of translated.matchAll(TOKEN_RE)) counts.set(Number(m[1]), (counts.get(Number(m[1])) ?? 0) + 1);
  const ok = protections.every((_p, i) => (counts.get(i) ?? 0) >= 1) && [...counts.keys()].every((i) => i < protections.length);
  if (!ok) return { text: translated, ok: false };
  return { text: translated.replace(TOKEN_RE, (_m, i) => protections[Number(i)].value), ok: true };
}
