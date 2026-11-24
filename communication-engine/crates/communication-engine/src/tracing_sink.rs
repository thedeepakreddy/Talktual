//! Optional bridge from CE observations to the `tracing` ecosystem.

use ce_core::observability::{Level, ObservabilitySink, Observation};

/// Forwards already-redacted observations to `tracing` under the
/// `communication_engine` target. Fields are rendered into one `fields`
/// attribute because `tracing` requires static field names.
#[derive(Debug, Default, Clone, Copy)]
pub struct TracingSink;

impl ObservabilitySink for TracingSink {
    fn record(&self, observation: &Observation) {
        match observation {
            Observation::Log {
                level,
                name,
                fields,
            } => {
                let rendered = fields
                    .iter()
                    .map(|f| format!("{}={}", f.key, f.value))
                    .collect::<Vec<_>>()
                    .join(" ");
                match level {
                    Level::Debug => {
                        tracing::debug!(target: "communication_engine", event = %name, fields = %rendered)
                    }
                    Level::Info => {
                        tracing::info!(target: "communication_engine", event = %name, fields = %rendered)
                    }
                    Level::Warn => {
                        tracing::warn!(target: "communication_engine", event = %name, fields = %rendered)
                    }
                    Level::Error => {
                        tracing::error!(target: "communication_engine", event = %name, fields = %rendered)
                    }
                }
            }
            Observation::Counter {
                name,
                delta,
                labels,
            } => {
                let rendered = labels
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                tracing::trace!(target: "communication_engine::metrics", metric = %name, delta, labels = %rendered);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_every_observation_kind_without_a_subscriber() {
        for level in [Level::Debug, Level::Info, Level::Warn, Level::Error] {
            TracingSink.record(&Observation::log(level, "test.event").field("k", "v"));
        }
        TracingSink.record(&Observation::counter("test.counter", 1).label("q", "x"));
    }
}
