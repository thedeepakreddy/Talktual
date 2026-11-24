//! The provider (adapter) contract.
//!
//! A provider is an external service or platform facility (a speech API, an
//! OS audio stack, an on-device model runtime) wrapped by an adapter. Vendor
//! types never cross this boundary: adapters translate vendor errors into
//! [`ProviderError`] and vendor capabilities into CE capability ids.
//!
//! Phase 1 ships no real provider. The test kit has a mock.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::BoxFuture;
use crate::capability::ExecutionLocation;
use crate::error::{CeError, ErrorCategory, Recoverability};
use crate::health::HealthStatus;
use crate::ids::{CapabilityId, ProviderId};
use crate::privacy::PrivacyFlags;

/// Static description of a provider adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    /// Stable identity.
    pub id: ProviderId,
    /// CE capabilities the provider can back.
    pub capabilities: Vec<CapabilityId>,
    /// Where processing happens.
    pub execution: Option<ExecutionLocation>,
    /// Privacy declarations.
    pub privacy: PrivacyFlags,
}

/// Normalized provider failure kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorKind {
    /// Temporarily unreachable or unavailable.
    Unavailable,
    /// Throttled; retry later.
    RateLimited,
    /// The provider did not answer in time.
    Timeout,
    /// Credentials missing, invalid or revoked.
    Unauthorized,
    /// The request was rejected as invalid.
    InvalidRequest,
    /// Quota or plan limits exhausted.
    QuotaExceeded,
    /// Unclassified provider failure.
    Internal,
}

/// A normalized provider failure. `message` must be operational text: never
/// echo request or response bodies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderError {
    /// Provider.
    pub provider: ProviderId,
    /// Kind.
    pub kind: ProviderErrorKind,
    /// Operational message.
    pub message: String,
    /// Provider-suggested retry delay.
    pub retry_after: Option<Duration>,
}

impl ProviderError {
    /// Recoverability implied by the kind.
    pub fn recoverability(&self) -> Recoverability {
        match self.kind {
            ProviderErrorKind::Unavailable
            | ProviderErrorKind::RateLimited
            | ProviderErrorKind::Timeout => Recoverability::Retryable,
            ProviderErrorKind::InvalidRequest => Recoverability::Recoverable,
            ProviderErrorKind::Unauthorized | ProviderErrorKind::QuotaExceeded => {
                Recoverability::Fatal
            }
            ProviderErrorKind::Internal => Recoverability::Unknown,
        }
    }

    /// Health implied by this failure.
    pub fn implied_health(&self) -> HealthStatus {
        match self.kind {
            ProviderErrorKind::RateLimited
            | ProviderErrorKind::Timeout
            | ProviderErrorKind::InvalidRequest => {
                HealthStatus::Degraded(format!("provider {:?}", self.kind))
            }
            ProviderErrorKind::Unavailable
            | ProviderErrorKind::Unauthorized
            | ProviderErrorKind::QuotaExceeded
            | ProviderErrorKind::Internal => {
                HealthStatus::Unhealthy(format!("provider {:?}", self.kind))
            }
        }
    }
}

impl From<ProviderError> for CeError {
    fn from(err: ProviderError) -> Self {
        let category = if err.kind == ProviderErrorKind::Timeout {
            ErrorCategory::Timeout
        } else {
            ErrorCategory::Provider
        };
        let code = match err.kind {
            ProviderErrorKind::Unavailable => "provider.unavailable",
            ProviderErrorKind::RateLimited => "provider.rate_limited",
            ProviderErrorKind::Timeout => "provider.timeout",
            ProviderErrorKind::Unauthorized => "provider.unauthorized",
            ProviderErrorKind::InvalidRequest => "provider.invalid_request",
            ProviderErrorKind::QuotaExceeded => "provider.quota_exceeded",
            ProviderErrorKind::Internal => "provider.internal",
        };
        let mut ce = CeError::new(category, code, err.message.clone())
            .with_context("provider", err.provider.to_string())
            .with_recoverability(err.recoverability());
        if let Some(after) = err.retry_after {
            ce = ce.with_context("retry_after_ms", after.as_millis().to_string());
        }
        ce
    }
}

/// An external provider adapter.
pub trait Provider: Send + Sync {
    /// Static description.
    fn descriptor(&self) -> ProviderDescriptor;
    /// Probe health. Must be bounded in time by the implementation.
    fn health(&self) -> BoxFuture<'_, Result<HealthStatus, ProviderError>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(kind: ProviderErrorKind) -> ProviderError {
        ProviderError {
            provider: ProviderId::new("ce.test.provider").unwrap(),
            kind,
            message: "request failed".into(),
            retry_after: Some(Duration::from_millis(250)),
        }
    }

    #[test]
    fn provider_errors_map_to_ce_categories_and_recoverability() {
        use ProviderErrorKind::*;
        let table = [
            (
                Unavailable,
                ErrorCategory::Provider,
                Recoverability::Retryable,
                "unhealthy",
            ),
            (
                RateLimited,
                ErrorCategory::Provider,
                Recoverability::Retryable,
                "degraded",
            ),
            (
                Timeout,
                ErrorCategory::Timeout,
                Recoverability::Retryable,
                "degraded",
            ),
            (
                Unauthorized,
                ErrorCategory::Provider,
                Recoverability::Fatal,
                "unhealthy",
            ),
            (
                InvalidRequest,
                ErrorCategory::Provider,
                Recoverability::Recoverable,
                "degraded",
            ),
            (
                QuotaExceeded,
                ErrorCategory::Provider,
                Recoverability::Fatal,
                "unhealthy",
            ),
            (
                Internal,
                ErrorCategory::Provider,
                Recoverability::Unknown,
                "unhealthy",
            ),
        ];
        for (kind, category, rec, health) in table {
            let e = err(kind);
            assert_eq!(e.implied_health().as_str(), health, "{kind:?}");
            let ce: CeError = e.into();
            assert_eq!(ce.category(), category, "{kind:?}");
            assert_eq!(ce.recoverability(), rec, "{kind:?}");
            assert!(ce.code().starts_with("provider."));
            assert!(ce.to_string().contains("retry_after_ms=250"));
        }
    }
}
