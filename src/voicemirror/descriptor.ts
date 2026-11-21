// Profile → descriptor (quantize, transmit) → render hints (listener side).

import { DESCRIPTOR_VERSION } from './types.ts';
import type { PitchRange, RenderHints, Register, VoiceDescriptor, VoiceProfile } from './types.ts';

/** A profile below this confidence is never turned into a descriptor (fallback: default voice). */
export const MIN_CONFIDENCE = 0.5;
/** Syllable-peak rate (per second of speech) that maps to tempo 1.0.
 *  From eval calibration on CALIBRATION speakers only (espeak 175 wpm → 5.94; see
 *  docs/phase3/PHASE3_EVALUATION_REPORT.md). The harness fails loudly if this drifts. */
export const REFERENCE_SYLLABLE_RATE = 5.9;

const clamp = (x: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, x));

export function toDescriptor(p: VoiceProfile): VoiceDescriptor | null {
  if (!(p.confidence >= MIN_CONFIDENCE) || !(p.f0MedianHz > 0)) return null;
  const st = 12 * Math.log2(p.f0MedianHz / 55);
  const pitchStep = clamp(Math.round(st / 2) * 2, 0, 44);
  const pitchRange: PitchRange = p.f0IqrSemitones < 1.5 ? 'flat' : p.f0IqrSemitones > 4 ? 'lively' : 'normal';
  const tempo = p.syllableRate > 0 ? p.syllableRate / REFERENCE_SYLLABLE_RATE : 1;
  const tempoPct = clamp(Math.round((tempo * 100) / 5) * 5, 80, 125);
  return { v: DESCRIPTOR_VERSION, pitchStep, pitchRange, tempoPct };
}

/** Wire validation: descriptors arrive from the other peer (untrusted). */
export function isVoiceDescriptor(x: unknown): x is VoiceDescriptor {
  if (!x || typeof x !== 'object') return false;
  const d = x as Record<string, unknown>;
  return (
    d.v === DESCRIPTOR_VERSION &&
    Number.isInteger(d.pitchStep) &&
    (d.pitchStep as number) >= 0 &&
    (d.pitchStep as number) <= 44 &&
    (d.pitchStep as number) % 2 === 0 &&
    (d.pitchRange === 'flat' || d.pitchRange === 'normal' || d.pitchRange === 'lively') &&
    Number.isInteger(d.tempoPct) &&
    (d.tempoPct as number) >= 80 &&
    (d.tempoPct as number) <= 125 &&
    (d.tempoPct as number) % 5 === 0 &&
    Object.keys(d).length === 4
  );
}

export const descriptorF0Hz = (d: VoiceDescriptor) => 55 * 2 ** (d.pitchStep / 12);

export function registerOf(f0Hz: number): Register {
  return f0Hz < 145 ? 'low' : f0Hz > 200 ? 'high' : 'mid';
}

/**
 * Listener-side mapping.
 * `voiceF0Hz` is the natural median pitch of the voice that will speak (known
 * for calibrated engines; ~ register default otherwise). Browser pitch is
 * relative to that voice and deliberately damped (×0.5 in semitones) and
 * clamped: large shifts hurt naturalness and intelligibility.
 */
export function toRenderHints(d: VoiceDescriptor, voiceF0Hz = 165): RenderHints {
  const target = descriptorF0Hz(d);
  const semitoneShift = 12 * Math.log2(target / voiceF0Hz);
  // Damped (half the semitone shift) frequency ratio relative to the voice's own pitch.
  const browserPitch = clamp(Math.pow(2, (semitoneShift * 0.5) / 12), 0.6, 1.6);
  const tempo = d.tempoPct / 100;
  return {
    register: registerOf(target),
    targetF0Hz: target,
    browserPitch: Math.round(browserPitch * 100) / 100,
    browserRate: clamp(tempo, 0.8, 1.25),
    ttsSpeed: clamp(tempo, 0.8, 1.2),
  };
}
