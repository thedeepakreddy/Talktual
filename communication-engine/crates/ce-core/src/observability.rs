//! Observability hooks with redaction.
//!
//! CE does not choose a logging or metrics backend. It reports structured
//! [`Observation`]s to an [`ObservabilitySink`] supplied by the host. Before an
//! observation reaches the sink, the runtime redacts every field whose
//! [`DataClass`] exceeds the configured [`RedactionPolicy`]
//! ([`Observation::redact`]). Secrets are never emitted.
//!
//! Sinks are called synchronously on control-plane tasks and must not block.
//! (Real-time audio paths must not call sinks at all; see realtime.md.)

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::privacy::{DataClass, RedactionPolicy};

/// Severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Verbose diagnostics.
    Debug,
    /// Normal operation.
    Info,
    /// Unexpected but handled.
    Warn,
    /// Failure.
    Error,
}

/// A classified field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// Field name.
    pub key: Cow<'static, str>,
    /// Field value (possibly a redaction marker after [`Observation::redact`]).
    pub value: String,
    /// Sensitivity.
    pub class: DataClass,
}

/// A structured log record or metric sample.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Observation {
    /// A log record.
    Log {
        /// Severity.
        level: Level,
        /// Stable event name, e.g. `module.state_changed`.
        name: Cow<'static, str>,
        /// Fields.
        fields: Vec<Field>,
    },
    /// A counter increment.
    Counter {
        /// Metric name, e.g. `ce.events.dropped`.
        name: Cow<'static, str>,
        /// Increment.
        delta: u64,
        /// Operational labels only.
        labels: Vec<(Cow<'static, str>, String)>,
    },
}

impl Observation {
    /// A log record with no fields.
    pub fn log(level: Level, name: impl Into<Cow<'static, str>>) -> Self {
        Observation::Log {
            level,
            name: name.into(),
            fields: Vec::new(),
        }
    }

    /// A counter increment.
    pub fn counter(name: impl Into<Cow<'static, str>>, delta: u64) -> Self {
        Observation::Counter {
            name: name.into(),
            delta,
            labels: Vec::new(),
        }
    }

    /// Add an operational field (no-op for counters).
    pub fn field(self, key: impl Into<Cow<'static, str>>, value: impl fmt::Display) -> Self {
        self.classified(key, value, DataClass::Operational)
    }

    /// Add a classified field (no-op for counters).
    pub fn classified(
        mut self,
        key: impl Into<Cow<'static, str>>,
        value: impl fmt::Display,
        class: DataClass,
    ) -> Self {
        if let Observation::Log { fields, .. } = &mut self {
            fields.push(Field {
                key: key.into(),
                value: value.to_string(),
                class,
            });
        }
        self
    }

    /// Add an operational label (no-op for logs).
    pub fn label(mut self, key: impl Into<Cow<'static, str>>, value: impl fmt::Display) -> Self {
        if let Observation::Counter { labels, .. } = &mut self {
            labels.push((key.into(), value.to_string()));
        }
        self
    }

    /// Replace every field value the policy does not permit with a marker.
    pub fn redact(mut self, policy: &RedactionPolicy) -> Self {
        if let Observation::Log { fields, .. } = &mut self {
            for f in fields.iter_mut() {
                if !policy.permits(f.class) {
                    f.value = format!("[redacted:{}]", f.class.as_str());
                }
            }
        }
        self
    }
}

/// Receives redacted observations. Must be cheap and non-blocking.
pub trait ObservabilitySink: Send + Sync + fmt::Debug {
    /// Record an observation.
    fn record(&self, observation: &Observation);
}

/// A sink that discards everything (the default).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopSink;

impl ObservabilitySink for NoopSink {
    fn record(&self, _observation: &Observation) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_replaces_values_above_policy() {
        let obs = Observation::log(Level::Info, "module.emitted")
            .field("module", "ce.mock.echo")
            .classified("language", "hu", DataClass::Personal)
            .classified("text", "hello", DataClass::Content)
            .classified("key", "sk-1", DataClass::Secret)
            .redact(&RedactionPolicy::DEFAULT);
        let Observation::Log { fields, .. } = obs else {
            panic!("expected log")
        };
        let values: Vec<_> = fields.iter().map(|f| f.value.as_str()).collect();
        assert_eq!(
            values,
            [
                "ce.mock.echo",
                "[redacted:personal]",
                "[redacted:content]",
                "[redacted:secret]"
            ]
        );
    }

    #[test]
    fn counters_take_labels_not_fields() {
        let c = Observation::counter("ce.events.dropped", 3)
            .label("queue", "command")
            .field("ignored", "x");
        assert_eq!(
            c,
            Observation::Counter {
                name: "ce.events.dropped".into(),
                delta: 3,
                labels: vec![("queue".into(), "command".into())]
            }
        );
        NoopSink.record(&c);
    }
}
