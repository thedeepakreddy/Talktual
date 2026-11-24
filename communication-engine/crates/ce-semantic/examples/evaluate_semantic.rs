//! Replay evaluation of `ce.semantic.rules` on `fixtures/semantic/`.
//!
//! ```text
//! cargo run --release -p ce-semantic --example evaluate_semantic -- [dev|test|asr|all] [--json] [--errors]
//! ```
//!
//! Prints identities (corpus/profile hashes, adapter version, config
//! fingerprint), per-split scores, streaming and robustness results, a
//! throughput/memory measurement and (with `--errors`) the error list.
//! Deterministic except for timings.

#![allow(clippy::print_stdout)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use ce_core::semantic::{DomainProfile, SemanticConfig};
use ce_semantic::eval::{Delivery, Prf, SplitReport, evaluate, fnv64, parse_corpus, replay};
use ce_semantic::rules::{RULES_ADAPTER_ID, RULES_VERSION};
use ce_semantic::{RulesFactory, SemanticFactory};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/semantic")
}

fn pct(x: f64) -> String {
    format!("{:.3}", x)
}

fn ratio(x: f64, support: u32) -> String {
    if support == 0 { "n/a".into() } else { pct(x) }
}

fn prf(p: &Prf) -> String {
    format!(
        "{} | {} | {} | {} | {} | {}",
        p.tp,
        p.fp,
        p.fn_,
        pct(p.precision),
        pct(p.recall),
        pct(p.f1)
    )
}

fn peak_rss_kib() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    s.lines()
        .find(|l| l.starts_with("VmHWM:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

fn print_split(r: &SplitReport, errors: bool) {
    println!(
        "\n## Split `{}` — {} dialogues, {} utterances\n",
        r.split, r.dialogues, r.utterances
    );
    for e in [&r.entities_all, &r.entities_medium] {
        println!(
            "### Entities (predictions with certainty ≥ {})\n",
            e.min_certainty
        );
        println!("| match | TP | FP | FN | P | R | F1 |\n|---|---|---|---|---|---|---|");
        println!("| exact span | {} |", prf(&e.exact));
        println!("| overlapping span | {} |", prf(&e.overlap));
        println!(
            "\n| category (exact) | TP | FP | FN | P | R | F1 |\n|---|---|---|---|---|---|---|"
        );
        for (k, v) in &e.per_category {
            println!("| {k} | {} |", prf(v));
        }
        println!(
            "\nNormalized values correct: {}/{} exact matches with a gold value. Lost to ASR (always FN): {}.\n",
            e.normalized_correct, e.normalized_checked, e.lost_to_asr
        );
    }
    let x = &r.references;
    println!("### References\n");
    println!(
        "gold {} · detected {} · correct {} · **accuracy {}** · spurious {} · predicted resolved/ambiguous/unresolved {}/{}/{} · resolution precision {} · abstention rate {}\n",
        x.gold,
        x.detected,
        x.correct,
        ratio(x.accuracy, x.gold),
        x.spurious,
        x.predicted_resolved,
        x.predicted_ambiguous,
        x.predicted_unresolved,
        ratio(x.resolution_precision, x.predicted_resolved),
        ratio(x.abstention_rate, x.detected)
    );
    println!("| gold \\ predicted | counts |\n|---|---|");
    for (g, m) in &x.confusion {
        println!(
            "| {g} | {} |",
            m.iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let a = &r.acts;
    println!("\n### Communicative acts (`ce.acts` 1.0.0)\n");
    println!(
        "utterances {} · **macro-F1 {}** · coverage {} · selective accuracy {} · gold abstentions {} · abstention recall {}\n",
        a.utterances,
        pct(a.macro_f1),
        pct(a.coverage),
        pct(a.selective_accuracy),
        a.gold_abstentions,
        ratio(a.abstention_recall, a.gold_abstentions)
    );
    println!("| label | TP | FP | FN | P | R | F1 |\n|---|---|---|---|---|---|---|");
    for (k, v) in &a.per_label {
        println!("| {k} | {} |", prf(v));
    }
    println!("\n| gold \\ predicted | counts |\n|---|---|");
    for (g, m) in &a.confusion {
        println!(
            "| {g} | {} |",
            m.iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let s = &r.streaming;
    println!("\n### Streaming (synthetic timeline: one revision per 300 ms)\n");
    println!(
        "inputs {} (partials {}) · observation events {:?} · events per input {} · retracted before final {} · finals {} (first seen in a partial: {})",
        s.inputs,
        s.partial_inputs,
        s.changes,
        pct(s.revision_rate),
        s.retracted_before_final,
        s.finals,
        s.finals_seen_early
    );
    println!(
        "first observation (audio ms) p50 {} p95 {} max {} (n={}) · processor time per input (ms) p50 {:.3} p95 {:.3} p99 {:.3} max {:.3} (n={})",
        s.first_observation_ms.p50,
        s.first_observation_ms.p95,
        s.first_observation_ms.max,
        s.first_observation_ms.count,
        s.processing_ms.p50,
        s.processing_ms.p95,
        s.processing_ms.p99,
        s.processing_ms.max,
        s.processing_ms.count
    );
    println!(
        "output per input (JSON bytes of semantic events) p50 {} p95 {} p99 {} max {}",
        s.output_bytes.p50, s.output_bytes.p95, s.output_bytes.p99, s.output_bytes.max
    );
    let b = &r.robustness;
    println!(
        "\n### Robustness\n\nduplicated + stale partials: {}/{} dialogues with identical finals ({} stale/duplicate inputs rejected) · finals only: {}/{} identical finals",
        b.disorder_consistent,
        b.disorder_dialogues,
        b.disorder_rejected,
        b.finals_only_consistent,
        b.finals_only_dialogues
    );
    if errors {
        println!("\n### Errors\n");
        for (d, u, k, t) in &r.errors {
            println!("* {d} u{u} {k}: {t}");
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let errors = args.iter().any(|a| a == "--errors");
    let which = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "all".into());
    let corpus_bytes = std::fs::read(root().join("corpus.jsonl")).expect("corpus");
    let corpus = parse_corpus(&String::from_utf8_lossy(&corpus_bytes)).expect("parse corpus");
    let mut profiles = BTreeMap::new();
    let mut profile_ids = Vec::new();
    for entry in std::fs::read_dir(root().join("profiles")).expect("profiles") {
        let path = entry.expect("entry").path();
        let bytes = std::fs::read(&path).expect("profile");
        let p: DomainProfile = serde_json::from_slice(&bytes).expect("profile json");
        p.validate().expect("valid profile");
        profile_ids.push(format!(
            "{}@{} file fnv {} profile fingerprint {}",
            p.id,
            p.version,
            fnv64(&bytes),
            p.fingerprint()
        ));
        profiles.insert(p.id.clone(), p);
    }
    let config = SemanticConfig::default();
    let factory = RulesFactory;
    println!("# Semantic replay evaluation\n");
    println!("* adapter: {RULES_ADAPTER_ID} {RULES_VERSION} (in process, deterministic)");
    println!(
        "* config fingerprint (default, no profile): {}",
        config.fingerprint()
    );
    println!(
        "* corpus: fixtures/semantic/corpus.jsonl, fnv {} ({} dialogues)",
        fnv64(&corpus_bytes),
        corpus.len()
    );
    for p in &profile_ids {
        println!("* profile: {p}");
    }
    println!(
        "* host: {} {} · {} logical CPUs",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0)
    );
    let splits: Vec<&str> = if which == "all" {
        vec!["dev", "test", "asr"]
    } else {
        vec![which.as_str()]
    };
    let mut reports = Vec::new();
    for s in splits {
        let r = evaluate(&factory, &config, &profiles, &corpus, s).expect("evaluate");
        if !json {
            print_split(&r, errors);
        }
        reports.push(r);
    }
    // Throughput and memory: replay every dialogue 200 times, single thread.
    let t0 = Instant::now();
    let mut inputs = 0u64;
    for _ in 0..200 {
        for d in &corpus {
            let r = replay(&factory, &config, &profiles, d, Delivery::Clean).expect("replay");
            inputs += r.input_ms.len() as u64;
        }
    }
    let secs = t0.elapsed().as_secs_f64();
    println!(
        "\n## Throughput\n\n{inputs} inputs in {:.3} s on one thread ({:.0} inputs/s, {:.1} µs per input incl. processor setup) · peak RSS of this process {} KiB",
        secs,
        inputs as f64 / secs,
        secs * 1e6 / inputs as f64,
        peak_rss_kib().map_or("n/a".into(), |k| k.to_string())
    );
    // Concurrency: 4 threads, each with its own processors, 50 corpus passes.
    let t0 = Instant::now();
    let total: u64 = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..4)
            .map(|_| {
                sc.spawn(|| {
                    let mut n = 0u64;
                    for _ in 0..50 {
                        for d in &corpus {
                            n += replay(&factory, &config, &profiles, d, Delivery::Clean)
                                .expect("replay")
                                .input_ms
                                .len() as u64;
                        }
                    }
                    n
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().expect("thread")).sum()
    });
    let secs = t0.elapsed().as_secs_f64();
    println!(
        "4 threads in parallel: {total} inputs in {:.3} s ({:.0} inputs/s aggregate) · peak RSS {} KiB",
        secs,
        total as f64 / secs,
        peak_rss_kib().map_or("n/a".into(), |k| k.to_string())
    );
    if json {
        println!("{}", serde_json::to_string_pretty(&reports.iter().map(|r| serde_json::json!({
            "split": r.split, "entities_all": r.entities_all, "entities_medium": r.entities_medium,
            "references": r.references, "acts": r.acts, "streaming": r.streaming, "robustness": r.robustness,
        })).collect::<Vec<_>>()).unwrap_or_default());
    }
    let _ = factory.capabilities();
}
