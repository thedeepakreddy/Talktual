// VoiceMirror contracts (Phase 3).
//
// Data classes, from most to least sensitive:
//   raw audio      — never stored, never leaves the analyzer (streamed, discarded)
//   VoiceProfile   — on-device summary statistics (Hz, rates). Never transmitted,
//                    never logged, cleared on stop/revoke/session end.
//   VoiceDescriptor— coarse, quantized, versioned; the ONLY thing sent to the peer,
//                    and only after the speaker opts in. Cannot reconstruct a voice.
//   RenderHints    — listener-side playback parameters derived from a descriptor.
//
// This module preserves *selected characteristics* (pitch register, pitch
// liveliness, tempo). It is not voice cloning and does not reproduce timbre;
// see docs/phase3/PHASE3_RESEARCH_AND_FEASIBILITY.md for measured limits.

export const DESCRIPTOR_VERSION = 1;

/** On-device only. */
export interface VoiceProfile {
  /** Median fundamental frequency of voiced frames, Hz. */
  f0MedianHz: number;
  /** Inter-quartile range of F0 in semitones (pitch liveliness). */
  f0IqrSemitones: number;
  /** Estimated syllable-like energy peaks per second of speech. */
  syllableRate: number;
  /** Seconds of voiced audio the estimate is based on. */
  voicedSeconds: number;
  /** 0..1 — how much to trust the profile (amount + periodicity of evidence). */
  confidence: number;
}

export type PitchRange = 'flat' | 'normal' | 'lively';

/** Transmitted (opt-in). Integers/enums only, coarse by construction. */
export interface VoiceDescriptor {
  v: typeof DESCRIPTOR_VERSION;
  /** Median pitch in semitones above 55 Hz, rounded to an even number (2-semitone steps), 0..44. */
  pitchStep: number;
  pitchRange: PitchRange;
  /** Speaking tempo relative to a reference rate, in 5% steps: 80..125 (= 0.80..1.25). */
  tempoPct: number;
}

/** Coarse voice register used to pick a stock voice when pitch can't be set. */
export type Register = 'low' | 'mid' | 'high';

/** Listener-side playback parameters. */
export interface RenderHints {
  register: Register;
  /** Target median pitch, Hz (for engines with calibrated pitch control). */
  targetF0Hz: number;
  /** SpeechSynthesisUtterance.pitch (0..2 range in the spec); clamped 0.6..1.6. */
  browserPitch: number;
  /** SpeechSynthesisUtterance.rate; clamped 0.8..1.25. */
  browserRate: number;
  /** Provider TTS speed multiplier; clamped 0.8..1.2. */
  ttsSpeed: number;
}

/** A renderer turns text + hints into audio. The interface accommodates the
 *  higher-fidelity tiers designed (not built) in Phase 3: catalog selection by
 *  on-device embedding, and consented provider cloning. */
export type VoiceTier = 'off' | 'characteristics' | 'stock-selection' | 'consented-clone';
