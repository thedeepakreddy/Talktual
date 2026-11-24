//! Audio measurement probe (Phase 2 preliminary measurement protocol).
//!
//! Opens a capture stream (and optionally playback) on a real backend via
//! cpal and reports: enumeration/open/start timings, delivered vs. expected
//! frames, block inter-arrival statistics, callback counts, overflow and
//! discontinuities, worker queue depth, and process CPU time and peak RSS
//! (Linux). See `docs/phases/phase-2-measurements.md` for methodology.
//!
//! Usage:
//!   cargo run --release -p communication-engine --features cpal --example audio_probe -- \
//!       [--device <name substring>] [--seconds N] [--rate HZ] [--channels N]
//!
//! It never stores or prints audio content: only counts and timings.

#![allow(clippy::print_stdout)]

#[cfg(not(feature = "cpal"))]
fn main() {
    println!("audio_probe needs the `cpal` feature");
}

#[cfg(feature = "cpal")]
#[tokio::main]
async fn main() -> std::process::ExitCode {
    match probe::run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            println!("probe failed: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(feature = "cpal")]
mod probe {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use communication_engine::audio::cpal_backend::CpalBackend;
    use communication_engine::contracts::time::{Clock, SystemClock};
    use communication_engine::prelude::*;

    struct Args {
        device: Option<String>,
        seconds: u64,
        rate: Option<u32>,
        channels: Option<u16>,
    }

    fn args() -> Args {
        let mut a = Args {
            device: None,
            seconds: 3,
            rate: None,
            channels: None,
        };
        let mut it = std::env::args().skip(1);
        while let Some(k) = it.next() {
            let v = it.next().unwrap_or_default();
            match k.as_str() {
                "--device" => a.device = Some(v),
                "--seconds" => a.seconds = v.parse().unwrap_or(3),
                "--rate" => a.rate = v.parse().ok(),
                "--channels" => a.channels = v.parse().ok(),
                _ => {}
            }
        }
        a
    }

    /// (user+system CPU seconds, peak RSS KiB) from /proc on Linux.
    fn process_usage() -> Option<(f64, u64)> {
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        let fields: Vec<&str> = stat.rsplit(')').next()?.split_whitespace().collect();
        let ticks: f64 =
            fields.get(11)?.parse::<f64>().ok()? + fields.get(12)?.parse::<f64>().ok()?;
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let hwm = status
            .lines()
            .find(|l| l.starts_with("VmHWM:"))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()?;
        Some((ticks / 100.0, hwm))
    }

    fn pct(sorted: &[f64], p: f64) -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        sorted[((sorted.len() - 1) as f64 * p).round() as usize]
    }

    pub(super) async fn run() -> Result<(), String> {
        let a = args();
        let e = |e: CeError| e.to_string();
        println!("host: {} {}", std::env::consts::OS, std::env::consts::ARCH);
        let engine = Engine::builder()
            .audio_backend(Arc::new(CpalBackend::default_host()))
            .build()
            .map_err(e)?;
        engine.start().await.map_err(e)?;
        let s = engine
            .create_session(SessionConfig::default())
            .await
            .map_err(e)?;
        engine
            .update_consent(
                s.id,
                ConsentPurpose::Capture,
                ConsentState::Granted {
                    at: SystemClock::new().wall(),
                },
            )
            .await
            .map_err(e)?;

        let t = Instant::now();
        let catalog = engine.audio_devices().await.map_err(e)?;
        println!(
            "backend: {}  enumerate_ms: {:.2}  devices: {}  problems: {}",
            catalog.backend.name,
            t.elapsed().as_secs_f64() * 1e3,
            catalog.devices.len(),
            catalog.problems.len()
        );
        let device = match &a.device {
            Some(sub) => catalog
                .devices
                .iter()
                .find(|d| d.supports_input && d.name.contains(sub.as_str())),
            None => catalog.default_input.and_then(|id| catalog.device(id)),
        }
        .ok_or("no matching input device (use --device <name substring>)")?;
        println!(
            "device: {} (input formats: {})",
            device.id,
            device.input_formats.len()
        );

        let request = StreamRequest {
            device: DeviceSelector::Device(device.id),
            format: FormatPreference {
                sample_rate: a.rate,
                channels: a.channels,
                ..Default::default()
            },
            ..StreamRequest::capture()
        };
        let t = Instant::now();
        let mut reader = engine.open_capture(s.id, request).await.map_err(e)?;
        let open_ms = t.elapsed().as_secs_f64() * 1e3;
        let info = reader.info().clone();
        let f = &info.format;
        println!(
            "resolved: {} Hz x{} {}  exact: {}  buffer requested/actual: {:?}/{:?}  block: {} frames  open_ms: {open_ms:.2}",
            f.device.sample_rate,
            f.device.channels,
            f.device.sample_format,
            f.is_exact(),
            f.requested_buffer_frames,
            f.actual_buffer_frames,
            info.block_frames
        );
        let usage_before = process_usage();
        let t = Instant::now();
        engine.start_audio(info.id).await.map_err(e)?;
        let start_ms = t.elapsed().as_secs_f64() * 1e3;
        let first = tokio::time::timeout(Duration::from_secs(5), reader.recv())
            .await
            .map_err(|_| "no audio within 5 s of start".to_owned())?
            .map_err(|r| format!("{r:?}"))?;
        let first_block_ms = t.elapsed().as_secs_f64() * 1e3;
        let run_start = Instant::now();
        let mut last = Instant::now();
        let mut gaps_ms = Vec::new();
        let mut frames = u64::from(first.meta.frames);
        let mut flagged = 0u64;
        let mut max_queued = 0usize;
        while run_start.elapsed() < Duration::from_secs(a.seconds) {
            match tokio::time::timeout(Duration::from_secs(2), reader.recv()).await {
                Ok(Ok(b)) => {
                    let now = Instant::now();
                    gaps_ms.push((now - last).as_secs_f64() * 1e3);
                    last = now;
                    frames += u64::from(b.meta.frames);
                    if b.meta.discontinuity.is_some() {
                        flagged += 1;
                    }
                    max_queued = max_queued.max(reader.queued());
                }
                Ok(Err(r)) => return Err(format!("stream ended: {r:?}")),
                Err(_) => return Err("stalled: no block for 2 s".into()),
            }
        }
        let wall = run_start.elapsed().as_secs_f64();
        let usage_after = process_usage();
        let status = engine.close_audio(info.id).await.map_err(e)?;
        gaps_ms.sort_by(f64::total_cmp);
        let expected = wall * f64::from(f.device.sample_rate);
        println!("start_ms: {start_ms:.2}  start_to_first_block_ms: {first_block_ms:.2}");
        println!(
            "run_s: {wall:.3}  frames: {frames}  frames/expected: {:.3}  callbacks: {}  frames/callback: {:.1}",
            frames as f64 / expected.max(1.0),
            status.counters.callbacks,
            status.counters.frames as f64 / status.counters.callbacks.max(1) as f64
        );
        println!(
            "block_interarrival_ms: p50 {:.2}  p99 {:.2}  max {:.2}  (n={})",
            pct(&gaps_ms, 0.5),
            pct(&gaps_ms, 0.99),
            gaps_ms.last().copied().unwrap_or(0.0),
            gaps_ms.len()
        );
        println!(
            "ring_overflow_frames: {}  queue_dropped_blocks: {}  discontinuities: {} (seen {flagged})  backend_xruns: {}  max_ring_fill: {}‰  max_queued_blocks: {max_queued}",
            status.counters.ring_overflow_frames,
            status.counters.queue_dropped_blocks,
            status.counters.discontinuities,
            status.counters.backend_xruns,
            status.counters.max_ring_fill_permille
        );
        if let (Some((c0, _)), Some((c1, hwm))) = (usage_before, usage_after) {
            println!(
                "process_cpu_s_during_run: {:.3} ({:.1}% of one core)  peak_rss_kib: {hwm}",
                c1 - c0,
                (c1 - c0) / wall * 100.0
            );
        }
        engine.shutdown().await.map_err(e)?;
        Ok(())
    }
}
