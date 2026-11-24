//! The CE error model.
//!
//! A [`CeError`] carries:
//!
//! * a typed [`ErrorCategory`] (what kind of failure),
//! * a stable machine-readable `code` (e.g. `registry.duplicate_module`),
//! * a human-readable `message` that **must contain only operational text** —
//!   never user content, secrets or personal data,
//! * classified context fields, each with a [`DataClass`], redacted on display,
//! * a [`Recoverability`] verdict,
//! * an optional source error, available programmatically through
//!   [`std::error::Error::source`] but **never** rendered by `Display`/`Debug`
//!   or copied into events (sources from providers may contain content).
//!
//! Use [`CeError::report`] to obtain the serializable, redacted form that
//! events and logs carry.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::privacy::{DataClass, RedactionPolicy};

/// Kind of failure. The set is closed for Phase 1 but marked non-exhaustive
/// so categories can be added without breaking consumers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCategory {
    /// Configuration is malformed, out of range or contains inline secrets.
    InvalidConfig,
    /// An argument or command failed validation.
    InvalidInput,
    /// The operation is not allowed in the current lifecycle state.
    Lifecycle,
    /// Registration, dependency, version or conflict resolution failed.
    CapabilityResolution,
    /// A bounded queue was full and the operation was rejected or dropped.
    QueueSaturated,
    /// The operation was cancelled.
    Cancelled,
    /// The operation did not complete in time.
    Timeout,
    /// An adapter or external provider failed.
    Provider,
    /// Encoding or decoding failed, including unsupported schema versions.
    Serialization,
    /// A bug or broken invariant inside CE or a module (including panics).
    Internal,
    /// The runtime is shutting down or has shut down.
    Shutdown,
}

impl ErrorCategory {
    /// Stable lowercase name.
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorCategory::InvalidConfig => "invalid_config",
            ErrorCategory::InvalidInput => "invalid_input",
            ErrorCategory::Lifecycle => "lifecycle",
            ErrorCategory::CapabilityResolution => "capability_resolution",
            ErrorCategory::QueueSaturated => "queue_saturated",
            ErrorCategory::Cancelled => "cancelled",
            ErrorCategory::Timeout => "timeout",
            ErrorCategory::Provider => "provider",
            ErrorCategory::Serialization => "serialization",
            ErrorCategory::Internal => "internal",
            ErrorCategory::Shutdown => "shutdown",
        }
    }

    /// The recoverability assumed for this category unless overridden.
    pub const fn default_recoverability(self) -> Recoverability {
        match self {
            ErrorCategory::QueueSaturated | ErrorCategory::Timeout => Recoverability::Retryable,
            ErrorCategory::InvalidConfig
            | ErrorCategory::InvalidInput
            | ErrorCategory::Lifecycle
            | ErrorCategory::CapabilityResolution
            | ErrorCategory::Cancelled
            | ErrorCategory::Serialization => Recoverability::Recoverable,
            ErrorCategory::Provider => Recoverability::Unknown,
            ErrorCategory::Internal | ErrorCategory::Shutdown => Recoverability::Fatal,
        }
    }
}

/// Whether, and how, a caller can recover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recoverability {
    /// Repeating the same operation later may succeed (transient condition).
    Retryable,
    /// Repeating the same operation will fail again, but the component or
    /// runtime remains usable (e.g. fix the input and try again).
    Recoverable,
    /// The component or runtime cannot continue and must be restarted.
    Fatal,
    /// Not known (e.g. an unclassified provider failure). Treat as not
    /// retryable unless a higher layer knows better.
    Unknown,
}

impl Recoverability {
    /// Whether an automatic retry is reasonable.
    pub const fn is_retryable(self) -> bool {
        matches!(self, Recoverability::Retryable)
    }
}

/// A classified context field attached to an error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextField {
    /// Field name (static, operational).
    pub key: Cow<'static, str>,
    /// Field value.
    pub value: String,
    /// Sensitivity of `value`.
    pub class: DataClass,
}

/// A CE error. Cheap to clone.
#[derive(Clone)]
pub struct CeError {
    category: ErrorCategory,
    code: Cow<'static, str>,
    message: Cow<'static, str>,
    context: Vec<ContextField>,
    recoverability: Recoverability,
    source: Option<Arc<dyn std::error::Error + Send + Sync + 'static>>,
}

impl CeError {
    /// Create an error. `code` must be a stable dotted identifier; `message`
    /// must contain operational text only.
    pub fn new(
        category: ErrorCategory,
        code: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            category,
            code: code.into(),
            message: message.into(),
            context: Vec::new(),
            recoverability: category.default_recoverability(),
            source: None,
        }
    }

    /// Attach an operational context field (shown by the default policy).
    pub fn with_context(self, key: impl Into<Cow<'static, str>>, value: impl Into<String>) -> Self {
        self.with_classified_context(key, value, DataClass::Operational)
    }

    /// Attach a context field with an explicit data class.
    pub fn with_classified_context(
        mut self,
        key: impl Into<Cow<'static, str>>,
        value: impl Into<String>,
        class: DataClass,
    ) -> Self {
        self.context.push(ContextField {
            key: key.into(),
            value: value.into(),
            class,
        });
        self
    }

    /// Override the category's default recoverability.
    pub fn with_recoverability(mut self, recoverability: Recoverability) -> Self {
        self.recoverability = recoverability;
        self
    }

    /// Attach an underlying error. It is never displayed or serialized.
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Arc::new(source));
        self
    }

    /// The category.
    pub fn category(&self) -> ErrorCategory {
        self.category
    }

    /// The stable code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// The operational message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Context fields (unredacted — apply a [`RedactionPolicy`] before
    /// exposing them).
    pub fn context(&self) -> &[ContextField] {
        &self.context
    }

    /// The recoverability verdict.
    pub fn recoverability(&self) -> Recoverability {
        self.recoverability
    }

    /// Whether an automatic retry is reasonable.
    pub fn is_retryable(&self) -> bool {
        self.recoverability.is_retryable()
    }

    /// Serializable, redacted representation for events and logs.
    pub fn report(&self, policy: &RedactionPolicy) -> ErrorReport {
        ErrorReport {
            category: self.category,
            code: self.code.to_string(),
            message: self.message.to_string(),
            recoverability: self.recoverability,
            context: self
                .context
                .iter()
                .map(|f| ReportField {
                    key: f.key.to_string(),
                    value: policy.apply(f.class, &f.value).into_owned(),
                })
                .collect(),
            has_source: self.source.is_some(),
        }
    }

    // ---- common constructors -------------------------------------------------

    /// Lifecycle violation.
    pub fn lifecycle(code: &'static str, message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCategory::Lifecycle, code, message)
    }

    /// Invalid configuration.
    pub fn invalid_config(code: &'static str, message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCategory::InvalidConfig, code, message)
    }

    /// Invalid input.
    pub fn invalid_input(code: &'static str, message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCategory::InvalidInput, code, message)
    }

    /// Internal invariant violation.
    pub fn internal(code: &'static str, message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCategory::Internal, code, message)
    }
}

impl fmt::Display for CeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let policy = RedactionPolicy::DEFAULT;
        write!(
            f,
            "[{}/{}] {}",
            self.category.as_str(),
            self.code,
            self.message
        )?;
        if !self.context.is_empty() {
            f.write_str(" (")?;
            for (i, field) in self.context.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write!(
                    f,
                    "{}={}",
                    field.key,
                    policy.apply(field.class, &field.value)
                )?;
            }
            f.write_str(")")?;
        }
        Ok(())
    }
}

impl fmt::Debug for CeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Debug output ends up in logs and panic messages, so it is redacted
        // exactly like Display and never renders the source.
        f.debug_struct("CeError")
            .field("display", &format_args!("{self}"))
            .field("recoverability", &self.recoverability)
            .field("has_source", &self.source.is_some())
            .finish()
    }
}

impl std::error::Error for CeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

/// Serializable, already-redacted error representation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorReport {
    /// Category.
    pub category: ErrorCategory,
    /// Stable code.
    pub code: String,
    /// Operational message.
    pub message: String,
    /// Recoverability.
    pub recoverability: Recoverability,
    /// Redacted context.
    pub context: Vec<ReportField>,
    /// Whether an underlying source error existed (its content is omitted).
    pub has_source: bool,
}

/// A redacted context field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportField {
    /// Field name.
    pub key: String,
    /// Value or redaction marker.
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[derive(Debug)]
    struct Leaky;
    impl fmt::Display for Leaky {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("provider echoed: my secret transcript")
        }
    }
    impl std::error::Error for Leaky {}

    fn sample() -> CeError {
        CeError::new(
            ErrorCategory::Provider,
            "provider.failed",
            "provider call failed",
        )
        .with_context("provider", "ce.mock.provider")
        .with_classified_context("utterance", "hello there", DataClass::Content)
        .with_classified_context("api_key", "sk-live-123", DataClass::Secret)
        .with_source(Leaky)
    }

    #[test]
    fn display_and_debug_redact_sensitive_context_and_never_render_sources() {
        let err = sample();
        for rendered in [err.to_string(), format!("{err:?}")] {
            assert!(rendered.contains("provider=ce.mock.provider"), "{rendered}");
            assert!(rendered.contains("[redacted:content]"), "{rendered}");
            assert!(rendered.contains("[redacted:secret]"), "{rendered}");
            assert!(!rendered.contains("hello there"), "{rendered}");
            assert!(!rendered.contains("sk-live"), "{rendered}");
            assert!(!rendered.contains("transcript"), "{rendered}");
        }
        // The source is still available programmatically.
        assert!(err.source().unwrap().to_string().contains("transcript"));
    }

    #[test]
    fn reports_are_redacted_and_serializable() {
        let report = sample().report(&RedactionPolicy::DEFAULT);
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("hello there") && !json.contains("sk-live"));
        assert!(json.contains("\"category\":\"provider\""));
        assert!(report.has_source);
        let permissive = sample().report(&RedactionPolicy {
            max_visible: DataClass::Content,
        });
        assert_eq!(permissive.context[1].value, "hello there");
        assert_eq!(permissive.context[2].value, "[redacted:secret]");
    }

    #[test]
    fn categories_map_to_default_recoverability_and_can_be_overridden() {
        use ErrorCategory::*;
        assert!(CeError::new(QueueSaturated, "q", "m").is_retryable());
        assert!(CeError::new(Timeout, "t", "m").is_retryable());
        assert_eq!(
            CeError::new(InvalidConfig, "c", "m").recoverability(),
            Recoverability::Recoverable
        );
        assert_eq!(
            CeError::new(Internal, "i", "m").recoverability(),
            Recoverability::Fatal
        );
        assert_eq!(
            CeError::new(Shutdown, "s", "m").recoverability(),
            Recoverability::Fatal
        );
        assert_eq!(
            CeError::new(Provider, "p", "m").recoverability(),
            Recoverability::Unknown
        );
        let overridden =
            CeError::new(Provider, "p", "m").with_recoverability(Recoverability::Retryable);
        assert!(overridden.is_retryable());
        for cat in [
            InvalidConfig,
            InvalidInput,
            Lifecycle,
            CapabilityResolution,
            QueueSaturated,
            Cancelled,
            Timeout,
            Provider,
            Serialization,
            Internal,
            Shutdown,
        ] {
            let json = serde_json::to_string(&cat).unwrap();
            assert_eq!(json, format!("\"{}\"", cat.as_str()));
        }
    }
}
