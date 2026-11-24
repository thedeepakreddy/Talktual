//! Real-ASR baseline evaluation: the sherpa-onnx streaming Zipformer
//! (INT8) through the **unchanged** Phase 4 evaluation pipeline.
//!
//! ```text
//! CE_SHERPA_MODEL_DIR=/path/to/model cargo run --release --example evaluate_real -- all
//! sections: isolated | pipeline | stability | formats | resources | partials | all
//! ```
//!
//! Evidence: **real model recognition** on **synthetic** (TTS) speech in
//! **replay**. Not human speech, not a device, not real time.

#![allow(clippy::print_stdout)]

use std::sync::Arc;
use std::time::Instant;

use ce_asr_sherpa::{SherpaConfig, SherpaFactory, model_dir_from_env};
use ce_audio_intel::eval::{Condition, Format};
use ce_core::speech::{
    LanguageHint, Percentiles, RevisionStatus, SegmentationMode, SpeechEvent, StabilityPolicy,
};
use ce_speech::WhichlangIdentifier;
use ce_speech::adapter::AsrFactory;
use ce_speech::eval::{self, ReplayOptions, SpeechMetrics, Split};
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
        "| {label} | {} | {} | {} / {} / {} | {:.3} ({}/{}) | {:.3} | {:.1} | {:.1} | {:.2} | {} | {} | {} | {} | {} | {:?} | {:.3} / {:.4} |",
        m.references,
        m.finals,
        m.missed_references,
        m.split_references,
        m.merged_finals,
        m.wer.rate(),
        m.wer.edits,
        m.wer.reference,
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

fn lid() -> Option<Arc<dyn LanguageIdentifier>> {
    Some(Arc::new(WhichlangIdentifier))
}

fn vm_hwm_kib() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    s.lines()
        .find(|l| l.starts_with("VmHWM:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
}

/// Each fixture utterance alone, fed in 100 ms chunks, then finalized.
fn isolated(f: &SherpaFactory) {
    println!("\n#### Isolated utterances (adapter only, clean, 100 ms chunks)\n");
    println!(
        "| split | id | reference | final hypothesis | WER | CER | first non-empty partial (audio ms) | partials | RTF |"
    );
    println!("|---|---|---|---|---|---|---|---|---|");
    let mut a = f.create().expect("adapter");
    a.load().expect("load");
    for split in [Split::Dev, Split::Test] {
        let mut tw = eval::ErrorRate::default();
        let mut tc = eval::ErrorRate::default();
        for (item, samples) in eval::load_split(split).expect("fixtures") {
            a.begin(0, &LanguageHint::Auto).expect("begin");
            let t0 = Instant::now();
            let mut first = None;
            let mut partials = 0;
            let mut fed = 0usize;
            for c in samples.chunks(1_600) {
                fed += c.len();
                if let Some(h) = a.accept(c).expect("accept") {
                    partials += 1;
                    if first.is_none() && !h.text.is_empty() {
                        first = Some(fed * 1_000 / 16_000);
                    }
                }
            }
            let fin = a.finalize().expect("finalize");
            let secs = t0.elapsed().as_secs_f64();
            let w = eval::wer(&item.reference, &fin.text);
            let c = eval::cer(&item.reference, &fin.text);
            tw = eval::total([tw, w]);
            tc = eval::total([tc, c]);
            println!(
                "| {} | {} | {} | {} | {:.3} | {:.3} | {} | {} | {:.3} |",
                item.split,
                item.id,
                item.reference,
                fin.text,
                w.rate(),
                c.rate(),
                first.map_or("–".into(), |v| v.to_string()),
                partials,
                secs / (samples.len() as f64 / 16_000.0)
            );
        }
        println!(
            "| {split:?} | **total** | | | **{:.3}** ({}/{}) | **{:.3}** ({}/{}) | | | |",
            tw.rate(),
            tw.edits,
            tw.reference,
            tc.rate(),
            tc.edits,
            tc.reference
        );
    }
}

fn conditions() -> Vec<Condition> {
    vec![
        Condition::Clean,
        Condition::White(20.0),
        Condition::White(10.0),
        Condition::White(5.0),
        Condition::Pink(10.0),
        Condition::Music(10.0),
        Condition::Clipped(20.0),
        Condition::Quiet(30.0),
    ]
}

fn pipeline(f: &SherpaFactory) {
    for split in [Split::Dev, Split::Test] {
        let utts = eval::load_split(split).expect("fixtures");
        println!(
            "\n#### Real model through the pipeline — {split:?} split (activity hints unless stated)\n"
        );
        header();
        for c in conditions() {
            let sc = eval::build("real", &utts, c, Format::BASE);
            let r = eval::replay(&sc, f, lid(), &ReplayOptions::default()).expect("replay");
            row(&c.label(), &eval::score(&sc, &r));
        }
        let sc = eval::build("real", &utts, Condition::Clean, Format::BASE);
        let mut o = ReplayOptions {
            analysis: None,
            ..Default::default()
        };
        o.recognition.segmentation = SegmentationMode::Continuous;
        let r = eval::replay(&sc, f, lid(), &o).expect("replay");
        row(
            "clean / continuous (model endpointing, no analysis)",
            &eval::score(&sc, &r),
        );
    }
}

/// Dev-only sweep of `lead_in_ms` (summed over three conditions).
fn lead_in(f: &SherpaFactory) {
    let utts = eval::load_split(Split::Dev).expect("fixtures");
    println!("\n#### lead_in_ms sweep — Dev only (clean + white 20 dB + pink 10 dB)\n");
    println!("| lead_in_ms | WER (edits/words) | CER | first partial p50 ms | start err p50 ms |");
    println!("|---|---|---|---|---|");
    let scs: Vec<_> = [
        Condition::Clean,
        Condition::White(20.0),
        Condition::Pink(10.0),
    ]
    .into_iter()
    .map(|c| eval::build("lead", &utts, c, Format::BASE))
    .collect();
    for lead in [0u32, 100, 200, 300, 400, 500, 600, 700, 800] {
        let (mut w, mut c) = (eval::ErrorRate::default(), eval::ErrorRate::default());
        let (mut fp, mut se) = (Vec::new(), Vec::new());
        for sc in &scs {
            let mut o = ReplayOptions::default();
            o.recognition.lead_in_ms = lead;
            let m = eval::score(sc, &eval::replay(sc, f, lid(), &o).expect("replay"));
            w = eval::total([w, m.wer]);
            c = eval::total([c, m.cer]);
            fp.push(m.first_partial_ms.p50);
            se.push(m.start_error_ms.p50);
        }
        println!(
            "| {lead} | {:.3} ({}/{}) | {:.3} | {:?} | {:?} |",
            w.rate(),
            w.edits,
            w.reference,
            c.rate(),
            fp,
            se
        );
    }
}

fn stability(f: &SherpaFactory) {
    let pol = |n, ms, hold| StabilityPolicy {
        stable_after_hypotheses: n,
        stable_after_ms: ms,
        holdback_words: hold,
    };
    let policies = [
        (
            "default 3 hyp / 800 ms / hold 1",
            StabilityPolicy::default(),
        ),
        ("2 hyp / off / hold 1", pol(2, 0, 1)),
        ("5 hyp / off / hold 1", pol(5, 0, 1)),
        ("off / 400 ms / hold 0", pol(0, 400, 0)),
        ("2 hyp / off / hold 0", pol(2, 0, 0)),
    ];
    for split in [Split::Dev, Split::Test] {
        let utts = eval::load_split(split).expect("fixtures");
        let sc = eval::build("stab", &utts, Condition::Clean, Format::BASE);
        println!("\n#### Commit policies with the real model — {split:?} (clean)\n");
        header();
        for (label, p) in &policies {
            if split == Split::Test && !label.starts_with("default") {
                continue;
            }
            let mut o = ReplayOptions::default();
            o.recognition.stability = p.clone();
            let r = eval::replay(&sc, f, lid(), &o).expect("replay");
            row(label, &eval::score(&sc, &r));
        }
    }
}

fn formats(f: &SherpaFactory) {
    let utts = eval::load_split(Split::Test).expect("fixtures");
    println!("\n#### Formats, drops and throttling — test split, real model\n");
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
        let sc = eval::build("fmt", &utts, Condition::Clean, fmt);
        let r = eval::replay(&sc, f, lid(), &ReplayOptions::default()).expect("replay");
        row(label, &eval::score(&sc, &r));
    }
    let sc = eval::build("fmt", &utts, Condition::Clean, Format::BASE);
    let o = ReplayOptions {
        drops: vec![(200, 10), (800, 10), (1_400, 10)],
        ..Default::default()
    };
    let r = eval::replay(&sc, f, lid(), &o).expect("replay");
    row("3 × 100 ms recognizer drops", &eval::score(&sc, &r));
    let mut o = ReplayOptions::default();
    o.recognition.budget.degrade = true;
    o.load = 2.0;
    let r = eval::replay(&sc, f, lid(), &o).expect("replay");
    row("throttled (forced load 2.0)", &eval::score(&sc, &r));
}

fn resources(f: &SherpaFactory) {
    println!("\n#### Resources and determinism (this machine)\n");
    let before = vm_hwm_kib();
    let t0 = Instant::now();
    let mut a = f.create().expect("adapter");
    let out = a.load().expect("load");
    let load_ms = t0.elapsed().as_millis();
    let after = vm_hwm_kib();
    println!("* model load (first, reused={}): {load_ms} ms", out.reused);
    let t1 = Instant::now();
    let mut b = f.create().expect("adapter");
    let out2 = b.load().expect("load");
    println!(
        "* second adapter load (reused={}): {} µs",
        out2.reused,
        t1.elapsed().as_micros()
    );
    println!(
        "* peak RSS before load {} KiB, after load {} KiB",
        before.map_or("?".into(), |v| v.to_string()),
        after.map_or("?".into(), |v| v.to_string())
    );
    let utts = eval::load_split(Split::Test).expect("fixtures");
    let sc = eval::build("res", &utts, Condition::Clean, Format::BASE);
    let r1 = eval::replay(&sc, f, lid(), &ReplayOptions::default()).expect("replay");
    let r2 = eval::replay(&sc, f, lid(), &ReplayOptions::default()).expect("replay");
    println!(
        "* test clean replay: audio {:.1} s, recognizer processing {:.2} s (RTF {:.3}), analyzer {:.3} s",
        r1.audio_s,
        r1.processing_s,
        r1.processing_s / r1.audio_s,
        r1.analysis_s
    );
    let (d1, d2) = (eval::digest(&r1.events), eval::digest(&r2.events));
    println!(
        "* determinism (num_threads = 1): digest {d1:016x} vs {d2:016x} → {}",
        if d1 == d2 { "identical" } else { "DIFFERENT" }
    );
    // Four streams decoded concurrently with one shared model.
    let t2 = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                eval::replay(&sc, f, lid(), &ReplayOptions::default()).expect("replay");
            });
        }
    });
    let wall = t2.elapsed().as_secs_f64();
    println!(
        "* 4 concurrent replays sharing one model: {:.2} s wall for 4 × {:.1} s audio (aggregate {:.3} s of wall per s of audio per stream)",
        wall,
        r1.audio_s,
        wall / r1.audio_s
    );
    println!(
        "* peak RSS of the whole evaluation process so far: {} KiB",
        vm_hwm_kib().map_or("?".into(), |v| v.to_string())
    );
}

fn partials(f: &SherpaFactory) {
    let utts = eval::load_split(Split::Test).expect("fixtures");
    let sc = eval::build("partials", &utts, Condition::Clean, Format::BASE);
    let r = eval::replay(&sc, f, lid(), &ReplayOptions::default()).expect("replay");
    println!(
        "\n#### Partial transcript behaviour — first two test utterances (real model, clean)\n"
    );
    println!("| utterance | rev | status | input ms | committed | provisional | amended |");
    println!("|---|---|---|---|---|---|---|");
    for e in &r.events {
        if let SpeechEvent::Transcript(t) = e {
            if t.meta.utterance > 1 {
                continue;
            }
            println!(
                "| {} | {} | {:?} | {} | {} | {} | {} |",
                t.meta.utterance,
                t.revision,
                t.status,
                t.meta.input_end_frame / 16,
                t.text.committed,
                t.text.provisional,
                if t.status == RevisionStatus::Final {
                    t.commit_amended.to_string()
                } else {
                    String::new()
                }
            );
        }
    }
}

fn main() {
    let Some(dir) = model_dir_from_env() else {
        println!("set CE_SHERPA_MODEL_DIR to the model directory");
        std::process::exit(2);
    };
    let f = SherpaFactory::new(SherpaConfig::zipformer_en_2023_06_26_int8(dir));
    let caps = f.capabilities();
    let mode = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    println!(
        "Evidence: REAL MODEL recognition ({} {}, {:?}) on SYNTHETIC TTS speech in REPLAY. Not human speech; no device.",
        caps.adapter,
        caps.version,
        caps.model
            .as_ref()
            .map(|m| (&m.id, &m.fingerprint, m.size_bytes))
    );
    let all = mode == "all";
    if all || mode == "resources" {
        resources(&f);
    }
    if all || mode == "isolated" {
        isolated(&f);
    }
    if all || mode == "partials" {
        partials(&f);
    }
    if all || mode == "leadin" {
        lead_in(&f);
    }
    if all || mode == "pipeline" {
        pipeline(&f);
    }
    if all || mode == "stability" {
        stability(&f);
    }
    if all || mode == "formats" {
        formats(&f);
    }
}
