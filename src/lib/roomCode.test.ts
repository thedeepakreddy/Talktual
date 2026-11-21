import { describe, it, expect } from 'vitest';
import { generateRoomCode, normalizeRoomCode, codeFromScan, CODE_LENGTH } from './roomCode';

describe('generateRoomCode (DEF-C05)', () => {
  it('is always exactly 6 uppercase alphanumerics', () => {
    for (let i = 0; i < 2000; i++) expect(generateRoomCode()).toMatch(/^[A-Z0-9]{6}$/);
  });

  it('matches the server session-id rule', () => {
    expect(generateRoomCode()).toMatch(/^[A-Z0-9]{4,12}$/);
  });

  it('rejects biased bytes and never returns a short code', () => {
    // A source that mostly yields bytes >= 252 (must be rejected).
    let calls = 0;
    const code = generateRoomCode((buf) => {
      calls++;
      buf.fill(calls < 3 ? 255 : 0);
      return buf;
    });
    expect(code).toBe('AAAAAA');
    expect(calls).toBe(3);
  });

  it('is not trivially repetitive', () => {
    const set = new Set(Array.from({ length: 500 }, () => generateRoomCode()));
    expect(set.size).toBe(500);
  });
});

describe('normalizeRoomCode / codeFromScan', () => {
  it('normalizes user and URL input', () => {
    expect(normalizeRoomCode('ab-c 12x9')).toBe('ABC12X');
    expect(normalizeRoomCode('<script>')).toBe('SCRIPT');
    expect(normalizeRoomCode(null)).toBe('');
    expect(normalizeRoomCode('a'.repeat(50))).toHaveLength(CODE_LENGTH);
  });

  it('extracts codes from QR payloads', () => {
    expect(codeFromScan('https://talktual.app/?join=ab12cd')).toBe('AB12CD');
    expect(codeFromScan('https://x/?a=1&join=AB12CD')).toBe('AB12CD');
    expect(codeFromScan('ZZ99ZZ')).toBe('ZZ99ZZ');
    expect(codeFromScan('https://x/?join=AB12CDEF')).toBeNull();
    expect(codeFromScan('hello world')).toBeNull();
  });
});
