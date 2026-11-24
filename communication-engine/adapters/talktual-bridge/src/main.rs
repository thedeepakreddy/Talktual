//! `talktual-bridge` — run the Talktual ↔ CE bridge with the sherpa-onnx
//! model.
//!
//! ```text
//! CE_SHERPA_MODEL_DIR=… talktual-bridge [--listen 127.0.0.1:8091] [--allow-origin http://localhost:5173]…
//! ```

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

use ce_asr_sherpa::{SherpaConfig, SherpaFactory, model_dir_from_env};
use ce_speech::AsrFactory;
use talktual_bridge::{Bridge, BridgeConfig};

#[tokio::main]
async fn main() -> ExitCode {
    let mut listen = "127.0.0.1:8091".to_owned();
    let mut cfg = BridgeConfig::default();
    let mut extra_origins = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--listen" => listen = args.next().unwrap_or(listen),
            "--allow-origin" => extra_origins.extend(args.next()),
            "--log-finals" => cfg.log_finals = args.next().map(Into::into),
            "-h" | "--help" => {
                println!(
                    "usage: talktual-bridge [--listen ADDR] [--allow-origin ORIGIN]... [--log-finals FILE.jsonl]\nenv: CE_SHERPA_MODEL_DIR"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown argument: {other}");
                return ExitCode::FAILURE;
            }
        }
    }
    cfg.allowed_origins.extend(extra_origins);
    let Some(dir) = model_dir_from_env() else {
        eprintln!("set CE_SHERPA_MODEL_DIR (see adapters/ce-asr-sherpa/scripts/fetch-model.sh)");
        return ExitCode::FAILURE;
    };
    let factory = SherpaFactory::new(SherpaConfig::zipformer_en_2023_06_26_int8(dir));
    // Keep one adapter loaded for the life of the process: every
    // recognition then reuses the model instead of loading it per push.
    let mut warm = match factory.create() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("model not usable: {e}");
            return ExitCode::FAILURE;
        }
    };
    let t0 = std::time::Instant::now();
    if let Err(e) = warm.load() {
        eprintln!("model load failed: {e}");
        return ExitCode::FAILURE;
    }
    println!("model loaded in {} ms", t0.elapsed().as_millis());
    let bridge = match Bridge::start(factory, cfg.clone()).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("engine failed to start: {e}");
            return ExitCode::FAILURE;
        }
    };
    let listener = match tokio::net::TcpListener::bind(&listen).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot listen on {listen}: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "talktual-bridge listening on ws://{listen} (origins: {:?})",
        cfg.allowed_origins
    );
    if let Some(p) = &cfg.log_finals {
        println!(
            "NOTE: transcripts are being written to {} (--log-finals)",
            p.display()
        );
    }
    tokio::select! {
        () = bridge.clone().serve(listener) => {}
        _ = tokio::signal::ctrl_c() => println!("shutting down"),
    }
    drop(warm);
    let _ = bridge.shutdown().await;
    ExitCode::SUCCESS
}
