//! Score a human test of Talktual + CE.
//!
//! ```text
//! cargo run --release --example score_human_test -- human-test/sentences-en.txt finals.jsonl
//! ```
//!
//! * `sentences-en.txt`: one sentence per line, in the order they were read.
//! * `finals.jsonl`: written by `talktual-bridge --log-finals finals.jsonl`.
//!
//! WER/CER use the same normalization as the Phase 4 evaluation
//! (`ce_speech::eval`), over the whole session (all finals in order vs all
//! sentences in order), so phrase splits and merges do not matter.
//! Latencies are wall-clock on your machine: first partial after the
//! utterance started, and the last final after you released push-to-talk.

#![allow(clippy::print_stdout)]

use ce_speech::eval::{cer, wer};
use ce_speech::metrics::Reservoir;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(refs), Some(log)) = (args.first(), args.get(1)) else {
        println!("usage: score_human_test SENTENCES.txt FINALS.jsonl");
        std::process::exit(2);
    };
    let reference: Vec<String> = std::fs::read_to_string(refs)
        .expect("read sentences")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    let mut texts = Vec::new();
    let (mut first, mut release) = (Reservoir::new(10_000), Reservoir::new(10_000));
    let mut audio_ms = 0u64;
    for line in std::fs::read_to_string(log).expect("read log").lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(t) = v["text"].as_str().filter(|t| !t.trim().is_empty()) {
            texts.push(t.to_owned());
        }
        if let Some(ms) = v["firstPartialAfterUtteranceStartMs"].as_u64() {
            first.record(ms as f32);
        }
        if let Some(ms) = v["finalAfterReleaseMs"].as_u64() {
            release.record(ms as f32);
        }
        audio_ms += v["audioMs"].as_u64().unwrap_or(0);
    }
    let (r, h) = (reference.join(" "), texts.join(" "));
    let (w, c) = (wer(&r, &h), cer(&r, &h));
    println!(
        "Evidence: REAL MODEL, live through Talktual and the bridge (wall-clock latencies on this machine). \
         Speech source: whatever reached the microphone; it is human speech only if a person read the sentences."
    );
    println!(
        "sentences: {}   finals: {}   audio recognized: {:.1} s",
        reference.len(),
        texts.len(),
        audio_ms as f64 / 1000.0
    );
    println!(
        "WER {:.3} ({}/{} words)   CER {:.3} ({}/{} chars)",
        w.rate(),
        w.edits,
        w.reference,
        c.rate(),
        c.edits,
        c.reference
    );
    let p = |x: ce_core::speech::Percentiles| {
        if x.count == 0 {
            "–".to_owned()
        } else {
            format!(
                "p50 {:.0} / p95 {:.0} / max {:.0} ms (n={})",
                x.p50, x.p95, x.max, x.count
            )
        }
    };
    println!(
        "first partial after utterance start: {}",
        p(first.summary())
    );
    println!(
        "final after release (push-to-talk):  {}",
        p(release.summary())
    );
    println!("\nreference: {r}\nrecognized: {h}");
}
