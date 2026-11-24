//! Audio configuration (part of `EngineConfig`).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::CeError;
use crate::resource::QueueCapacity;

/// What the stream worker does when the capture delivery queue is full.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryOverflowPolicy {
    /// Evict the oldest queued block (keeps latency bounded and the newest
    /// audio; the block after the gap is flagged). Default for live capture.
    DropOldest,
    /// Discard the newly produced block (keeps the queued prefix intact; the
    /// next delivered block is flagged).
    DropNewest,
}

/// Audio limits and policies. Every buffer is bounded; see
/// `docs/architecture/audio.md` for the memory bound each implies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioConfig {
    /// Capture block length in ms (frames per block = rate × ms / 1000).
    /// Default 10.
    pub block_ms: u32,
    /// Capacity of the capture device ring in ms of audio. Default 250.
    pub capture_ring_ms: u32,
    /// Capture delivery queue capacity, in blocks. Default 64.
    pub capture_queue_blocks: QueueCapacity,
    /// Policy when the capture delivery queue is full. Default drop oldest.
    pub capture_queue_overflow: DeliveryOverflowPolicy,
    /// Capacity of the playback device ring in ms of audio. Default 250.
    pub playback_ring_ms: u32,
    /// Playback write queue capacity, in blocks. Default 64.
    pub playback_queue_blocks: QueueCapacity,
    /// Maximum frames in one playback write, in ms of audio. Default 1000.
    pub playback_max_write_ms: u32,
    /// Worker poll interval in ms (how often the ring is drained). Default 2.
    pub worker_poll_ms: u32,
    /// Timeout for opening a device stream, in ms. Default 5000.
    pub open_timeout_ms: u64,
    /// Timeout for start/stop/close of a stream, in ms. Default 2000.
    pub operation_timeout_ms: u64,
    /// Timeout for device enumeration, in ms. Default 5000.
    pub enumerate_timeout_ms: u64,
    /// Maximum concurrently open streams. Default 8.
    pub max_streams: u32,
    /// Maximum age of capture consent; older consent is stale. Default none.
    pub consent_max_age_ms: Option<u64>,
    /// Re-enumerate devices this often to detect changes (backends without
    /// hotplug notifications). Default none (on request only).
    pub device_poll_interval_ms: Option<u64>,
    /// Minimum interval between coalesced pressure events per stream, in ms.
    /// Default 250.
    pub pressure_report_interval_ms: u64,
    /// Interval of audio housekeeping (staleness, zombie reaping), in ms.
    /// Default 500.
    pub housekeeping_interval_ms: u64,
}

impl Default for AudioConfig {
    fn default() -> Self {
        let cap = |n| QueueCapacity::new(n).expect("default capacities are valid");
        Self {
            block_ms: 10,
            capture_ring_ms: 250,
            capture_queue_blocks: cap(64),
            capture_queue_overflow: DeliveryOverflowPolicy::DropOldest,
            playback_ring_ms: 250,
            playback_queue_blocks: cap(64),
            playback_max_write_ms: 1_000,
            worker_poll_ms: 2,
            open_timeout_ms: 5_000,
            operation_timeout_ms: 2_000,
            enumerate_timeout_ms: 5_000,
            max_streams: 8,
            consent_max_age_ms: None,
            device_poll_interval_ms: None,
            pressure_report_interval_ms: 250,
            housekeeping_interval_ms: 500,
        }
    }
}

impl AudioConfig {
    /// Validate ranges and relationships.
    pub fn validate(&self) -> Result<(), CeError> {
        let range = |name: &'static str, value: u64, min: u64, max: u64| {
            if (min..=max).contains(&value) {
                Ok(())
            } else {
                Err(CeError::invalid_config(
                    "config.out_of_range",
                    "configuration value out of range",
                )
                .with_context("field", name)
                .with_context("value", value.to_string())
                .with_context("allowed", format!("{min}..={max}")))
            }
        };
        range("audio.block_ms", u64::from(self.block_ms), 1, 100)?;
        range(
            "audio.capture_ring_ms",
            u64::from(self.capture_ring_ms),
            20,
            5_000,
        )?;
        range(
            "audio.playback_ring_ms",
            u64::from(self.playback_ring_ms),
            20,
            5_000,
        )?;
        range(
            "audio.playback_max_write_ms",
            u64::from(self.playback_max_write_ms),
            1,
            5_000,
        )?;
        range(
            "audio.worker_poll_ms",
            u64::from(self.worker_poll_ms),
            1,
            50,
        )?;
        range("audio.open_timeout_ms", self.open_timeout_ms, 1, 60_000)?;
        range(
            "audio.operation_timeout_ms",
            self.operation_timeout_ms,
            1,
            60_000,
        )?;
        range(
            "audio.enumerate_timeout_ms",
            self.enumerate_timeout_ms,
            1,
            60_000,
        )?;
        range("audio.max_streams", u64::from(self.max_streams), 1, 64)?;
        range(
            "audio.pressure_report_interval_ms",
            self.pressure_report_interval_ms,
            10,
            60_000,
        )?;
        range(
            "audio.housekeeping_interval_ms",
            self.housekeeping_interval_ms,
            50,
            60_000,
        )?;
        if let Some(age) = self.consent_max_age_ms {
            range("audio.consent_max_age_ms", age, 1, 30 * 24 * 3_600_000)?;
        }
        if let Some(poll) = self.device_poll_interval_ms {
            range("audio.device_poll_interval_ms", poll, 100, 3_600_000)?;
        }
        if self.capture_ring_ms < 2 * self.block_ms {
            return Err(CeError::invalid_config(
                "config.audio_ring_too_small",
                "the capture ring must hold at least two blocks",
            ));
        }
        Ok(())
    }

    /// Open timeout.
    pub fn open_timeout(&self) -> Duration {
        Duration::from_millis(self.open_timeout_ms)
    }

    /// Start/stop/close timeout.
    pub fn operation_timeout(&self) -> Duration {
        Duration::from_millis(self.operation_timeout_ms)
    }

    /// Enumeration timeout.
    pub fn enumerate_timeout(&self) -> Duration {
        Duration::from_millis(self.enumerate_timeout_ms)
    }

    /// Consent max age.
    pub fn consent_max_age(&self) -> Option<Duration> {
        self.consent_max_age_ms.map(Duration::from_millis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate_and_bad_values_are_rejected() {
        AudioConfig::default().validate().unwrap();
        let c = AudioConfig {
            worker_poll_ms: 0,
            ..AudioConfig::default()
        };
        assert_eq!(c.validate().unwrap_err().code(), "config.out_of_range");
        let c = AudioConfig {
            block_ms: 50,
            capture_ring_ms: 60,
            ..AudioConfig::default()
        };
        assert_eq!(
            c.validate().unwrap_err().code(),
            "config.audio_ring_too_small"
        );
        let c = AudioConfig {
            device_poll_interval_ms: Some(5),
            ..AudioConfig::default()
        };
        assert!(c.validate().is_err());
        let parsed: AudioConfig = serde_json::from_str(
            r#"{"capture_queue_overflow":"drop_newest","consent_max_age_ms":60000}"#,
        )
        .unwrap();
        assert_eq!(
            parsed.capture_queue_overflow,
            DeliveryOverflowPolicy::DropNewest
        );
        assert_eq!(parsed.consent_max_age(), Some(Duration::from_secs(60)));
        assert!(serde_json::from_str::<AudioConfig>(r#"{"ring":1}"#).is_err());
    }
}
