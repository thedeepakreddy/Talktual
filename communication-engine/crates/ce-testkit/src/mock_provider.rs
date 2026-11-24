use std::sync::Mutex;

use ce_core::BoxFuture;
use ce_core::capability::ExecutionLocation;
use ce_core::health::HealthStatus;
use ce_core::ids::{CapabilityId, ProviderId};
use ce_core::privacy::PrivacyFlags;
use ce_core::provider::{Provider, ProviderDescriptor, ProviderError, ProviderErrorKind};

/// A provider adapter whose health is scripted by the test. It makes no
/// network calls and holds no credentials.
#[derive(Debug)]
pub struct MockProvider {
    descriptor: ProviderDescriptor,
    state: Mutex<Result<HealthStatus, ProviderError>>,
}

impl MockProvider {
    /// A healthy on-device mock backing `capabilities`.
    pub fn new(id: &str, capabilities: Vec<CapabilityId>) -> Self {
        Self {
            descriptor: ProviderDescriptor {
                id: ProviderId::new(id).expect("valid provider id"),
                capabilities,
                execution: Some(ExecutionLocation::OnDevice),
                privacy: PrivacyFlags::default(),
            },
            state: Mutex::new(Ok(HealthStatus::Healthy)),
        }
    }

    /// Script the next health result.
    pub fn set_health(&self, health: HealthStatus) {
        *self.state.lock().unwrap_or_else(|p| p.into_inner()) = Ok(health);
    }

    /// Script a failure of the given kind.
    pub fn fail_with(&self, kind: ProviderErrorKind) {
        *self.state.lock().unwrap_or_else(|p| p.into_inner()) = Err(ProviderError {
            provider: self.descriptor.id.clone(),
            kind,
            message: "scripted mock failure".into(),
            retry_after: None,
        });
    }

    /// Current scripted state, synchronously.
    pub fn current(&self) -> Result<HealthStatus, ProviderError> {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl Provider for MockProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn health(&self) -> BoxFuture<'_, Result<HealthStatus, ProviderError>> {
        let state = self.current();
        Box::pin(async move { state })
    }
}
