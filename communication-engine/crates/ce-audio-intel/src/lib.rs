//! Communication Engine audio intelligence (Phase 3).
//!
//! Consumes Phase 2 [`AudioBlock`](ce_core::audio::AudioBlock)s and produces
//! sparse, privacy-limited analysis events: voice activity, segment and
//! endpoint candidates, silence windows, quality, a small set of acoustic
//! conditions, adaptive-policy decisions and resource degradation.
//!
//! Pipeline: [`framer`] (blocks → hops + gaps) → [`vad`] (scores) →
//! [`activity`] (hysteresis) → [`segment`] (segments, endpoints, silence)
//! alongside [`quality`] (metrics, usability, conditions) and [`policy`]
//! (adaptive and resource decisions), assembled by [`analyzer`] and run on a
//! supervised thread by [`worker`]. [`eval`] provides replay and metrics.
//!
//! Nothing here infers who is speaking, what is said, emotion, health or any
//! personal trait. Outputs describe the **signal**, never the person.

#![forbid(unsafe_code)]

pub mod activity;
pub mod analyzer;
pub mod dsp;
pub mod eval;
pub mod framer;
pub mod policy;
pub mod quality;
pub mod segment;
pub mod synth;
pub mod vad;
pub mod worker;

pub use analyzer::StreamAnalyzer;
pub use vad::{EnergyVad, ScriptedVad, SpectralVad, VadObservation, VoiceActivityDetector};

#[cfg(test)]
pub(crate) mod testutil {
    pub(crate) use crate::synth::{block, noise, sine, speechlike};

    use crate::framer::{Hop, HopFeatures};
    use ce_core::time::MonotonicTime;

    /// A 10 ms hop at 16 kHz with index `i` and -40 dBFS level.
    pub(crate) fn hop(i: u64) -> Hop {
        Hop {
            index: i,
            frame_start: i * 160,
            frames: 160,
            time: MonotonicTime::from_nanos(i * 10_000_000),
            end_time: MonotonicTime::from_nanos((i + 1) * 10_000_000),
            sequence: i,
            audio: (0, 0),
            features: HopFeatures {
                level_db: -40.0,
                mean_square: 1e-4,
                samples: 160,
                channels: 1,
                ..HopFeatures::default()
            },
        }
    }
}
