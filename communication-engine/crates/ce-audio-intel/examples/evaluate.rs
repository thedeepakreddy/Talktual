//! Evaluate detectors on the bundled fixtures.
//!
//! ```text
//! cargo run --release -p ce-audio-intel --example evaluate            # test split, all conditions
//! cargo run --release -p ce-audio-intel --example evaluate -- dev     # development split
//! cargo run --release -p ce-audio-intel --example evaluate -- formats # format/channel variants (test)
//! cargo run --release -p ce-audio-intel --example evaluate -- tune    # weight grid on dev only
//! ```

#![allow(clippy::print_stdout)]

use ce_audio_intel::eval::{self, Condition, Format, Split};
use ce_audio_intel::vad::{SpectralVad, SpectralWeights};
use ce_core::intel::{AnalysisConfig, VadAlgorithm};

fn row(label: &str, m: &eval::Metrics) {
    println!(
        "| {label} | {:.3} | {:.3} | {:.3} | {:.0} | {} | {:.0} | {:.0} | {:.0} | {:.2} | {} | {}/{} | {} | {:.0} | {:?} | {:.4} |",
        m.precision,
        m.recall,
        m.f1,
        m.false_activations_per_hour,
        m.missed_ms,
        m.onset_error_ms,
        m.offset_error_ms,
        m.onset_latency_ms,
        m.fragmentation,
        m.merges,
        m.endpoint_hits,
        m.endpoints,
        m.false_endpoints,
        m.endpoint_delay_ms,
        m.final_usability,
        m.real_time_factor,
    );
}

fn header() {
    println!(
        "| scenario | P | R | F1 | FA/h | missed ms | onset err | offset err | onset lat | frag | merges | endpoints | false EP | EP delay | usability | RTF |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "test".into());
    let split = if mode == "dev" || mode == "tune" {
        Split::Dev
    } else {
        Split::Test
    };
    let utts = eval::load_split(split).expect("fixtures");
    if mode == "tune" {
        let mut best = (f32::MIN, SpectralWeights::V1);
        for snr_mid in [6.0, 9.0, 12.0] {
            for periodicity in [2.0, 5.0] {
                for flatness in [3.0, 6.0] {
                    for modulation in [0.0, 2.0, 4.0] {
                        let w = SpectralWeights {
                            snr_mid,
                            periodicity,
                            flatness,
                            modulation,
                            ..SpectralWeights::V1
                        };
                        let cfg = AnalysisConfig::default();
                        let mut total = 0.0;
                        for c in eval::standard_conditions() {
                            let s = eval::build("dev", &utts, c, Format::BASE);
                            let r = eval::replay(
                                &s,
                                &cfg,
                                Some(Box::new(SpectralVad::with_weights(cfg.hop_ms, w))),
                            )
                            .unwrap();
                            let m = eval::score(&s, &r, &cfg);
                            // F1 minus a penalty of 0.01 per false activation per minute.
                            total += m.f1 - 0.01 * m.false_activations_per_hour / 60.0;
                        }
                        println!(
                            "snr_mid {snr_mid} periodicity {periodicity} flatness {flatness} modulation {modulation}: {total:.4}"
                        );
                        if total > best.0 {
                            best = (total, w);
                        }
                    }
                }
            }
        }
        println!("best: {:?} ({:.4})", best.1, best.0);
        return;
    }
    for vad in [VadAlgorithm::Spectral, VadAlgorithm::Energy] {
        let cfg = AnalysisConfig {
            vad,
            ..AnalysisConfig::default()
        };
        println!("\n### {} — {:?} split\n", vad.id(), split);
        header();
        if mode == "formats" {
            for f in [
                Format::BASE,
                Format {
                    rate: 8_000,
                    channels: 1,
                    block_frames: 80,
                },
                Format {
                    rate: 44_100,
                    channels: 1,
                    block_frames: 441,
                },
                Format {
                    rate: 48_000,
                    channels: 2,
                    block_frames: 480,
                },
                Format {
                    rate: 48_000,
                    channels: 1,
                    block_frames: 1024,
                },
            ] {
                let s = eval::build("test", &utts, Condition::White(10.0), f);
                let r = eval::replay(&s, &cfg, None).unwrap();
                row(
                    &format!("{} Hz x{} / {} fr", f.rate, f.channels, f.block_frames),
                    &eval::score(&s, &r, &cfg),
                );
            }
        } else {
            for c in eval::standard_conditions() {
                let s = eval::build("split", &utts, c, Format::BASE);
                let r = eval::replay(&s, &cfg, None).unwrap();
                row(&c.label(), &eval::score(&s, &r, &cfg));
            }
        }
    }
}
