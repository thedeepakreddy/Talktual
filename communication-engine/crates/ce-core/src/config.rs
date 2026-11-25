//! Typed, validated engine configuration.
//!
//! * **Defaults:** every field has a documented default ([`RuntimeConfig::default`]).
//! * **Overrides:** configuration deserializes with `#[serde(default)]`, so a
//!   partial document overrides only the fields it names. Unknown fields are
//!   rejected to catch typos.
//! * **Versioning:** [`EngineConfig::config_version`] must equal
//!   [`CONFIG_VERSION`]. A breaking change to the schema increments it.
//! * **No secrets:** module configuration may reference secrets only through
//!   [`Value::Secret`]; plain strings under secret-looking keys are rejected.
//!
//! CE does not read files or environment variables itself; the host chooses
//! the source and format.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::CeError;
use crate::ids::ModuleId;
use crate::privacy::RedactionPolicy;
use crate::resource::{AdmissionPolicy, OverflowPolicy, QueueCapacity};
use crate::value::Value;

/// The configuration schema version this crate understands.
pub const CONFIG_VERSION: u32 = 1;

/// Key fragments that indicate a secret.
const SECRET_KEY_HINTS: &[&str] = &[
    "secret",
    "password",
    "passwd",
    "token",
    "api_key",
    "apikey",
    "private_key",
    "credential",
];

/// Top-level configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EngineConfig {
    /// Schema version; must equal [`CONFIG_VERSION`].
    pub config_version: u32,
    /// Runtime limits and policies.
    pub runtime: RuntimeConfig,
    /// Per-module configuration, keyed by module id.
    pub modules: BTreeMap<ModuleId, ModuleConfig>,
    /// Audio limits and policies (Phase 2).
    pub audio: crate::audio::AudioConfig,
    /// Default audio-analysis configuration (Phase 3).
    pub analysis: crate::intel::AnalysisConfig,
    /// Speech recognition and language settings (Phase 4).
    pub speech: crate::speech::SpeechConfig,
    /// Semantic interpretation settings (Phase 5).
    pub semantic: crate::semantic::SemanticSettings,
    /// Translation settings (Phase 6).
    pub translation: crate::translation::TranslationSettings,
    /// Orchestration settings (Phase 7).
    pub orchestration: crate::orchestration::OrchestrationSettings,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            config_version: CONFIG_VERSION,
            runtime: RuntimeConfig::default(),
            modules: BTreeMap::new(),
            audio: crate::audio::AudioConfig::default(),
            analysis: crate::intel::AnalysisConfig::default(),
            speech: crate::speech::SpeechConfig::default(),
            semantic: crate::semantic::SemanticSettings::default(),
            translation: crate::translation::TranslationSettings::default(),
            orchestration: crate::orchestration::OrchestrationSettings::default(),
        }
    }
}

impl EngineConfig {
    /// Validate the whole configuration.
    pub fn validate(&self) -> Result<(), CeError> {
        if self.config_version != CONFIG_VERSION {
            return Err(CeError::invalid_config(
                "config.unsupported_version",
                "unsupported configuration version",
            )
            .with_context("found", self.config_version.to_string())
            .with_context("supported", CONFIG_VERSION.to_string()));
        }
        self.runtime.validate()?;
        self.audio.validate()?;
        self.analysis.validate()?;
        self.speech.validate()?;
        self.semantic.validate()?;
        self.translation.validate()?;
        self.orchestration.validate()?;
        for (id, module) in &self.modules {
            module
                .check_no_inline_secrets()
                .map_err(|e| e.with_context("module", id.to_string()))?;
        }
        Ok(())
    }

    /// Configuration for `module`, or an empty configuration.
    pub fn module(&self, module: &ModuleId) -> ModuleConfig {
        self.modules.get(module).cloned().unwrap_or_default()
    }
}

/// Runtime limits and policies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeConfig {
    /// Capacity of the command queue. Default 64.
    pub command_queue_capacity: QueueCapacity,
    /// Behaviour when the command queue is full. Default: wait up to 500 ms.
    pub command_admission: AdmissionPolicy,
    /// Capacity of the module-event ingress queue. Default 256.
    pub event_ingress_capacity: QueueCapacity,
    /// Events retained for each external subscriber before the oldest are
    /// overwritten (the subscriber is then told how many it missed).
    /// Default 1024.
    pub event_broadcast_capacity: QueueCapacity,
    /// Capacity of each module's event mailbox. Default 128.
    pub module_mailbox_capacity: QueueCapacity,
    /// Behaviour when a module mailbox is full. Default: drop newest.
    pub module_mailbox_overflow: OverflowPolicy,
    /// Capacity of each module's supervised-task spawn queue. Default 16.
    pub module_task_queue_capacity: QueueCapacity,
    /// Maximum supervised background tasks per module. Default 32.
    pub max_tasks_per_module: u32,
    /// Timeout for each module lifecycle operation, in ms. Default 5000.
    pub module_operation_timeout_ms: u64,
    /// Upper bound on the whole shutdown, in ms. Default 10000.
    pub shutdown_timeout_ms: u64,
    /// Maximum concurrently open sessions. Default 64.
    pub max_sessions: u32,
    /// What may appear in logs and error displays. Default: operational only.
    pub redaction: RedactionPolicy,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        let cap = |n| QueueCapacity::new(n).expect("default capacities are valid");
        Self {
            command_queue_capacity: cap(64),
            command_admission: AdmissionPolicy::WaitUpTo { timeout_ms: 500 },
            event_ingress_capacity: cap(256),
            event_broadcast_capacity: cap(1024),
            module_mailbox_capacity: cap(128),
            module_mailbox_overflow: OverflowPolicy::DropNewest,
            module_task_queue_capacity: cap(16),
            max_tasks_per_module: 32,
            module_operation_timeout_ms: 5_000,
            shutdown_timeout_ms: 10_000,
            max_sessions: 64,
            redaction: RedactionPolicy::DEFAULT,
        }
    }
}

const MAX_TIMEOUT_MS: u64 = 10 * 60 * 1000;

impl RuntimeConfig {
    /// Validate ranges. Queue capacities are already range-checked by type.
    pub fn validate(&self) -> Result<(), CeError> {
        let range = |name: &'static str, value: u64, min: u64, max: u64| {
            if value < min || value > max {
                Err(CeError::invalid_config(
                    "config.out_of_range",
                    "configuration value out of range",
                )
                .with_context("field", name)
                .with_context("value", value.to_string())
                .with_context("allowed", format!("{min}..={max}")))
            } else {
                Ok(())
            }
        };
        range(
            "module_operation_timeout_ms",
            self.module_operation_timeout_ms,
            1,
            MAX_TIMEOUT_MS,
        )?;
        range(
            "shutdown_timeout_ms",
            self.shutdown_timeout_ms,
            1,
            MAX_TIMEOUT_MS,
        )?;
        range("max_sessions", u64::from(self.max_sessions), 1, 100_000)?;
        range(
            "max_tasks_per_module",
            u64::from(self.max_tasks_per_module),
            1,
            10_000,
        )?;
        if let AdmissionPolicy::WaitUpTo { timeout_ms } = self.command_admission {
            range(
                "command_admission.timeout_ms",
                timeout_ms,
                1,
                MAX_TIMEOUT_MS,
            )?;
        }
        Ok(())
    }

    /// Module operation timeout.
    pub fn module_operation_timeout(&self) -> Duration {
        Duration::from_millis(self.module_operation_timeout_ms)
    }

    /// Overall shutdown timeout.
    pub fn shutdown_timeout(&self) -> Duration {
        Duration::from_millis(self.shutdown_timeout_ms)
    }
}

/// Opaque, module-defined configuration. Validated by the module's factory.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct ModuleConfig(pub BTreeMap<String, Value>);

impl ModuleConfig {
    /// Value for `key`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    /// Builder-style insert.
    pub fn with(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.0.insert(key.into(), value.into());
        self
    }

    /// Reject plain values under keys that look like secrets, at any depth.
    /// Secrets must be supplied as [`Value::Secret`] references.
    pub fn check_no_inline_secrets(&self) -> Result<(), CeError> {
        fn walk(path: &str, key: &str, value: &Value) -> Result<(), CeError> {
            let lower = key.to_ascii_lowercase();
            let looks_secret = SECRET_KEY_HINTS.iter().any(|h| lower.contains(h));
            match value {
                Value::Secret(_) | Value::Null => Ok(()),
                Value::Map(map) => map.iter().try_for_each(|(k, v)| walk(&format!("{path}.{k}"), k, v)),
                Value::List(items) => items
                    .iter()
                    .enumerate()
                    .try_for_each(|(i, v)| walk(&format!("{path}[{i}]"), key, v)),
                _ if looks_secret => Err(CeError::invalid_config(
                    "config.inline_secret",
                    "secret-like configuration keys must use a secret reference, not a literal value",
                )
                .with_context("key", path.trim_start_matches('.').to_owned())),
                _ => Ok(()),
            }
        }
        self.0.iter().try_for_each(|(k, v)| walk(k, k, v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCategory;
    use crate::value::SecretRef;

    #[test]
    fn defaults_are_valid_and_documented_values() {
        let cfg = EngineConfig::default();
        cfg.validate().unwrap();
        assert_eq!(cfg.config_version, CONFIG_VERSION);
        assert_eq!(cfg.runtime.command_queue_capacity.get(), 64);
        assert_eq!(
            cfg.runtime.module_operation_timeout(),
            Duration::from_secs(5)
        );
        assert_eq!(cfg.runtime.redaction, RedactionPolicy::DEFAULT);
    }

    #[test]
    fn partial_documents_override_only_named_fields() {
        let cfg: EngineConfig = serde_json::from_str(
            r#"{"runtime":{"max_sessions":3,"command_admission":{"policy":"reject"}},
                "modules":{"ce.mock.echo":{"greeting":"hi","retries":2}}}"#,
        )
        .unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.runtime.max_sessions, 3);
        assert_eq!(cfg.runtime.command_admission, AdmissionPolicy::Reject);
        assert_eq!(cfg.runtime.event_ingress_capacity.get(), 256);
        let module = cfg.module(&ModuleId::new("ce.mock.echo").unwrap());
        assert_eq!(module.get("retries"), Some(&Value::Integer(2)));
        assert_eq!(
            cfg.module(&ModuleId::new("ce.other").unwrap()),
            ModuleConfig::default()
        );
    }

    #[test]
    fn unknown_fields_bad_capacities_and_unknown_versions_are_rejected() {
        assert!(serde_json::from_str::<EngineConfig>(r#"{"runtime":{"max_sesions":3}}"#).is_err());
        assert!(
            serde_json::from_str::<EngineConfig>(r#"{"runtime":{"module_mailbox_capacity":0}}"#)
                .is_err()
        );
        assert!(serde_json::from_str::<EngineConfig>(r#"{"modules":{"Bad Id":{}}}"#).is_err());
        let cfg: EngineConfig = serde_json::from_str(r#"{"config_version":2}"#).unwrap();
        let err = cfg.validate().unwrap_err();
        assert_eq!(err.code(), "config.unsupported_version");
    }

    #[test]
    fn out_of_range_values_are_rejected() {
        for patch in [
            r#"{"runtime":{"module_operation_timeout_ms":0}}"#,
            r#"{"runtime":{"shutdown_timeout_ms":99999999}}"#,
            r#"{"runtime":{"max_sessions":0}}"#,
            r#"{"runtime":{"max_tasks_per_module":0}}"#,
            r#"{"runtime":{"command_admission":{"policy":"wait_up_to","timeout_ms":0}}}"#,
        ] {
            let cfg: EngineConfig = serde_json::from_str(patch).unwrap();
            let err = cfg.validate().unwrap_err();
            assert_eq!(err.category(), ErrorCategory::InvalidConfig, "{patch}");
            assert_eq!(err.code(), "config.out_of_range");
        }
    }

    #[test]
    fn inline_secrets_are_rejected_at_any_depth_and_references_are_accepted() {
        let inline = ModuleConfig::default().with("api_key", "sk-123");
        let err = inline.check_no_inline_secrets().unwrap_err();
        assert_eq!(err.code(), "config.inline_secret");
        assert!(!err.to_string().contains("sk-123"));

        let mut nested = BTreeMap::new();
        nested.insert("auth_token".to_owned(), Value::from("abc"));
        let deep = ModuleConfig::default().with("provider", Value::Map(nested));
        let err = deep.check_no_inline_secrets().unwrap_err();
        assert!(err.to_string().contains("provider.auth_token"), "{err}");

        let list = ModuleConfig::default().with("tokens", Value::List(vec![Value::from("t1")]));
        assert!(list.check_no_inline_secrets().is_err());

        let reference = ModuleConfig::default().with(
            "api_key",
            Value::Secret(SecretRef {
                secret_ref: "PROVIDER_KEY".into(),
            }),
        );
        reference.check_no_inline_secrets().unwrap();

        let mut engine = EngineConfig::default();
        engine
            .modules
            .insert(ModuleId::new("ce.mock.echo").unwrap(), inline);
        assert_eq!(
            engine.validate().unwrap_err().code(),
            "config.inline_secret"
        );
    }
}
