//! Phase 4 evaluation tables (Markdown).
//!
//! ```text
//! cargo run --release -p ce-speech --example evaluate_speech -- lid        # language identification
//! cargo run --release -p ce-speech --example evaluate_speech -- pipeline   # oracle pipeline, test split
//! cargo run --release -p ce-speech --example evaluate_speech -- stability  # commit policies (dev), default (test)
//! cargo run --release -p ce-speech --example evaluate_speech -- robustness # formats, drops, throttling (test)
//! cargo run --release -p ce-speech --example evaluate_speech -- all
//! ```
//!
//! Every number is **synthetic replay** (TTS fixtures, generated noise)
//! with the **oracle** reference adapter unless stated: it measures the CE
//! pipeline, never speech-recognition accuracy.

#![allow(clippy::print_stdout)]

use std::sync::Arc;

use ce_audio_intel::eval::{Condition, Format};
use ce_core::speech::{LanguageIdConfig, Percentiles, SegmentationMode, StabilityPolicy};
use ce_speech::WhichlangIdentifier;
use ce_speech::eval::{self, ReplayOptions, SpeechMetrics, Split};
use ce_speech::fake::{ReferenceFactory, ReferenceNoise};
use ce_speech::language::LanguageIdentifier;

fn p(x: &Percentiles) -> String {
    if x.count == 0 {
        "–".into()
    } else {
        format!("{:.0} / {:.0} / {:.0}", x.p50, x.p95, x.max)
    }
}

fn header() {
    println!(
        "| scenario | refs | finals | missed / split / merged | WER | CER | partials/s | rev/utt | corrections/utt | amended | first partial ms p50/p95/max | first commit ms | final ms | start err ms | LID (utterance) | RTF rec / ana |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
}

fn row(label: &str, m: &SpeechMetrics) {
    println!(
        "| {label} | {} | {} | {} / {} / {} | {:.3} | {:.3} | {:.1} | {:.1} | {:.2} | {} | {} | {} | {} | {} | {:?} | {:.4} / {:.4} |",
        m.references,
        m.finals,
        m.missed_references,
        m.split_references,
        m.merged_finals,
        m.wer.rate(),
        m.cer.rate(),
        m.partial_rate_per_s,
        m.revisions_per_utterance,
        m.corrections_per_utterance,
        m.commit_amendments,
        p(&m.first_partial_ms),
        p(&m.first_commit_ms),
        p(&m.final_ms),
        p(&m.start_error_ms),
        m.languages,
        m.rtf,
        m.analysis_rtf,
    );
}

fn lid() {
    let items = eval::load_lid_corpus().expect("corpus");
    let w = WhichlangIdentifier;
    for (label, cfg) in [
        (
            "default (min_letters 10, split check on)",
            LanguageIdConfig::default(),
        ),
        (
            "split check off",
            LanguageIdConfig {
                split_check: false,
                ..LanguageIdConfig::default()
            },
        ),
        (
            "no evidence gate (min_letters 0, split check off)",
            LanguageIdConfig {
                min_letters: 0,
                split_check: false,
                ..LanguageIdConfig::default()
            },
        ),
    ] {
        let r = eval::evaluate_lid(&items, &w, &cfg);
        println!("\n#### {} {} — {label}\n", w.id(), w.version());
        println!("| split | length | n | decided | coverage | accuracy (decided) |");
        println!("|---|---|---|---|---|---|");
        for ((split, len), c) in &r.groups {
            println!(
                "| {split} | {len} | {} | {} | {:.2} | {:.3} |",
                c.n,
                c.decided,
                c.coverage(),
                c.accuracy()
            );
        }
        let mixed: Vec<String> = r.mixed.iter().map(|(id, o)| format!("{id}: {o}")).collect();
        println!("\nMixed (test): {}", mixed.join("; "));
        if label.starts_with("default") {
            println!("\nConfusion (test, truth → outcome):\n");
            for ((split, t, o), n) in &r.confusion {
                if split == "test" && t != o {
                    println!("* {t} → {o}: {n}");
                }
            }
        }
    }
}

fn reference(sc: &eval::SpeechScenario, noise: ReferenceNoise) -> ReferenceFactory {
    ReferenceFactory::new(sc.references.clone(), noise)
}

fn lid_arc() -> Option<Arc<dyn LanguageIdentifier>> {
    Some(Arc::new(WhichlangIdentifier))
}

fn pipeline() {
    for split in [Split::Dev, Split::Test] {
        let utts = eval::load_split(split).expect("fixtures");
        println!("\n#### Oracle pipeline — {split:?} split\n");
        header();
        for c in [
            Condition::Clean,
            Condition::White(20.0),
            Condition::White(10.0),
            Condition::White(5.0),
            Condition::Pink(10.0),
            Condition::Music(10.0),
            Condition::Clipped(20.0),
            Condition::Quiet(30.0),
        ] {
            let sc = eval::build("p", &utts, c, Format::BASE);
            let f = reference(&sc, ReferenceNoise::default());
            for (mode, seg) in [
                ("hints", SegmentationMode::ActivityHints),
                ("continuous", SegmentationMode::Continuous),
            ] {
                if mode == "continuous" && c != Condition::Clean {
                    continue;
                }
                let mut o = ReplayOptions::default();
                o.recognition.segmentation = seg;
                let r = eval::replay(&sc, &f, lid_arc(), &o).expect("replay");
                row(&format!("{} / {mode}", c.label()), &eval::score(&sc, &r));
            }
        }
    }
}

fn policies() -> Vec<(&'static str, StabilityPolicy)> {
    let p = |n, ms, hold| StabilityPolicy {
        stable_after_hypotheses: n,
        stable_after_ms: ms,
        holdback_words: hold,
    };
    vec![
        (
            "default 3 hyp / 800 ms / hold 1",
            StabilityPolicy::default(),
        ),
        ("2 hyp / off / hold 1", p(2, 0, 1)),
        ("3 hyp / off / hold 1", p(3, 0, 1)),
        ("5 hyp / off / hold 1", p(5, 0, 1)),
        ("off / 400 ms / hold 0", p(0, 400, 0)),
        ("off / 800 ms / hold 1", p(0, 800, 1)),
        ("2 hyp / off / hold 0", p(2, 0, 0)),
    ]
}

fn stability() {
    let noise = ReferenceNoise {
        truncate_rate: 0.3,
        rewrite_rate: 0.15,
        substitution_rate: 0.0,
        seed: 7,
    };
    for split in [Split::Dev, Split::Test] {
        let utts = eval::load_split(split).expect("fixtures");
        let sc = eval::build("s", &utts, Condition::Clean, Format::BASE);
        let f = reference(&sc, noise);
        println!(
            "\n#### Commit policies with revision noise (truncate 0.3, rewrite 0.15) — {split:?}\n"
        );
        header();
        for (label, pol) in policies() {
            if split == Split::Test && !label.starts_with("default") {
                continue; // held out: only the chosen policy is reported
            }
            let mut o = ReplayOptions::default();
            o.recognition.stability = pol;
            let r = eval::replay(&sc, &f, lid_arc(), &o).expect("replay");
            row(label, &eval::score(&sc, &r));
        }
    }
}

fn robustness() {
    let utts = eval::load_split(Split::Test).expect("fixtures");
    println!("\n#### Robustness — test split, oracle\n");
    header();
    for (label, fmt) in [
        ("16 kHz mono 10 ms", Format::BASE),
        (
            "48 kHz stereo 10 ms",
            Format {
                rate: 48_000,
                channels: 2,
                block_frames: 480,
            },
        ),
        (
            "44.1 kHz mono 512-frame blocks",
            Format {
                rate: 44_100,
                channels: 1,
                block_frames: 512,
            },
        ),
        (
            "8 kHz mono 20 ms",
            Format {
                rate: 8_000,
                channels: 1,
                block_frames: 160,
            },
        ),
    ] {
        let sc = eval::build("r", &utts, Condition::Clean, fmt);
        let f = reference(&sc, ReferenceNoise::default());
        let r = eval::replay(&sc, &f, lid_arc(), &ReplayOptions::default()).expect("replay");
        row(label, &eval::score(&sc, &r));
    }
    let sc = eval::build("r", &utts, Condition::Clean, Format::BASE);
    let f = reference(&sc, ReferenceNoise::default());
    // 100 ms of the recognizer's audio dropped inside each of the first
    // utterances (blocks at 2.0 s, 8.0 s, 14.0 s).
    let o = ReplayOptions {
        drops: vec![(200, 10), (800, 10), (1_400, 10)],
        ..Default::default()
    };
    let r = eval::replay(&sc, &f, lid_arc(), &o).expect("replay");
    row("3 × 100 ms recognizer drops", &eval::score(&sc, &r));
    let mut o = ReplayOptions::default();
    o.recognition.budget.degrade = true;
    o.load = 2.0;
    let r = eval::replay(&sc, &f, lid_arc(), &o).expect("replay");
    row("throttled (forced load 2.0)", &eval::score(&sc, &r));
    let mut o = ReplayOptions {
        analysis: None,
        ..Default::default()
    };
    o.recognition.segmentation = SegmentationMode::Continuous;
    o.recognition.finalization.max_utterance_ms = 5_000;
    let r = eval::replay(&sc, &f, lid_arc(), &o).expect("replay");
    row("no analysis, continuous, 5 s max", &eval::score(&sc, &r));
    let a = eval::replay(&sc, &f, lid_arc(), &ReplayOptions::default()).expect("replay");
    let b = eval::replay(&sc, &f, lid_arc(), &ReplayOptions::default()).expect("replay");
    println!(
        "\nReplay determinism: digest {:016x} vs {:016x} → {}",
        eval::digest(&a.events),
        eval::digest(&b.events),
        if eval::digest(&a.events) == eval::digest(&b.events) {
            "identical"
        } else {
            "DIFFERENT"
        }
    );
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    println!(
        "Evidence: synthetic replay; transcripts from the oracle reference adapter (not ASR)."
    );
    if mode == "lid" || mode == "all" {
        lid();
    }
    if mode == "pipeline" || mode == "all" {
        pipeline();
    }
    if mode == "stability" || mode == "all" {
        stability();
    }
    if mode == "robustness" || mode == "all" {
        robustness();
    }
}
