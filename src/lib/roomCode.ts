// Channel codes. The code is the only thing standing between a stranger and a
// call, so it comes from the CSPRNG and is always exactly CODE_LENGTH long.

export const CODE_LENGTH = 6;
const ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

export function generateRoomCode(random: (buf: Uint8Array) => Uint8Array = (b) => crypto.getRandomValues(b)): string {
  // Rejection sampling keeps every character uniformly likely: 252 is the
  // largest multiple of 36 that fits in a byte.
  const limit = 256 - (256 % ALPHABET.length);
  let out = "";
  while (out.length < CODE_LENGTH) {
    for (const byte of random(new Uint8Array(CODE_LENGTH * 2))) {
      if (byte < limit) out += ALPHABET[byte % ALPHABET.length];
      if (out.length === CODE_LENGTH) break;
    }
  }
  return out;
}

/** Uppercases and strips anything that can't be part of a code. */
export function normalizeRoomCode(raw: string | null | undefined): string {
  return String(raw ?? "")
    .toUpperCase()
    .replace(/[^A-Z0-9]/g, "")
    .slice(0, CODE_LENGTH);
}

/** Extracts a code from a scanned QR payload: a join URL or a bare code. */
export function codeFromScan(raw: string): string | null {
  const urlMatch = raw.match(/[?&]join=([a-zA-Z0-9]{6})(?![a-zA-Z0-9])/);
  if (urlMatch) return urlMatch[1].toUpperCase();
  const trimmed = raw.trim();
  return /^[a-zA-Z0-9]{6}$/.test(trimmed) ? trimmed.toUpperCase() : null;
}
