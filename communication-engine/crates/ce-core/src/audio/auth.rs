//! The capture authorization gate.
//!
//! Capture requires **both** of:
//!
//! 1. CE consent: the session's `ConsentPurpose::Capture` is `Granted`, and
//!    not older than the configured maximum age (if any);
//! 2. platform permission: the backend (or the host, on its behalf) reports
//!    `Granted`, or `NotRequiredByPlatform`.
//!
//! Everything else — absent, denied, revoked, stale, not determined,
//! restricted or unknown — denies. Opening a device is never treated as
//! permission to capture.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::device::PlatformPermission;
use crate::privacy::ConsentState;
use crate::time::WallTime;

/// Why capture is not authorized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationDenial {
    /// No consent recorded for capture.
    ConsentAbsent,
    /// The user declined.
    ConsentDenied,
    /// The user withdrew consent.
    ConsentRevoked,
    /// Consent is older than the configured maximum age.
    ConsentStale,
    /// The platform denied access.
    PermissionDenied,
    /// The platform has not asked the user yet.
    PermissionNotDetermined,
    /// The platform restricts access.
    PermissionRestricted,
    /// The platform state cannot be determined.
    PermissionUnknown,
}

impl AuthorizationDenial {
    /// Stable lowercase name.
    pub const fn as_str(self) -> &'static str {
        match self {
            AuthorizationDenial::ConsentAbsent => "consent_absent",
            AuthorizationDenial::ConsentDenied => "consent_denied",
            AuthorizationDenial::ConsentRevoked => "consent_revoked",
            AuthorizationDenial::ConsentStale => "consent_stale",
            AuthorizationDenial::PermissionDenied => "permission_denied",
            AuthorizationDenial::PermissionNotDetermined => "permission_not_determined",
            AuthorizationDenial::PermissionRestricted => "permission_restricted",
            AuthorizationDenial::PermissionUnknown => "permission_unknown",
        }
    }
}

/// Result of evaluating the gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision", content = "denial")]
pub enum CaptureAuthorization {
    /// Capture may proceed.
    Granted,
    /// Capture must not proceed (or must stop).
    Denied(AuthorizationDenial),
}

impl CaptureAuthorization {
    /// Whether capture may proceed.
    pub const fn is_granted(self) -> bool {
        matches!(self, CaptureAuthorization::Granted)
    }
}

/// Evaluate the gate. Consent is checked first, then platform permission.
pub fn evaluate_capture_authorization(
    permission: PlatformPermission,
    consent: ConsentState,
    now: WallTime,
    max_consent_age: Option<Duration>,
) -> CaptureAuthorization {
    use AuthorizationDenial as D;
    match consent {
        ConsentState::Unknown => return CaptureAuthorization::Denied(D::ConsentAbsent),
        ConsentState::Denied => return CaptureAuthorization::Denied(D::ConsentDenied),
        ConsentState::Revoked { .. } => return CaptureAuthorization::Denied(D::ConsentRevoked),
        ConsentState::Granted { at } => {
            if let Some(max) = max_consent_age {
                let age = Duration::from_nanos(now.unix_nanos().saturating_sub(at.unix_nanos()));
                if age > max {
                    return CaptureAuthorization::Denied(D::ConsentStale);
                }
            }
        }
    }
    match permission {
        PlatformPermission::Granted | PlatformPermission::NotRequiredByPlatform => {
            CaptureAuthorization::Granted
        }
        PlatformPermission::Denied => CaptureAuthorization::Denied(D::PermissionDenied),
        PlatformPermission::NotDetermined => {
            CaptureAuthorization::Denied(D::PermissionNotDetermined)
        }
        PlatformPermission::Restricted => CaptureAuthorization::Denied(D::PermissionRestricted),
        PlatformPermission::Unknown => CaptureAuthorization::Denied(D::PermissionUnknown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: WallTime = WallTime::from_unix_nanos(1_000_000_000_000);

    fn granted(ago: Duration) -> ConsentState {
        ConsentState::Granted {
            at: WallTime::from_unix_nanos(
                NOW.unix_nanos() - u64::try_from(ago.as_nanos()).unwrap(),
            ),
        }
    }

    #[test]
    fn denies_by_default_and_requires_both_consent_and_permission() {
        use AuthorizationDenial as D;
        use PlatformPermission as P;
        let fresh = granted(Duration::from_secs(1));
        let cases = [
            (
                P::Granted,
                ConsentState::Unknown,
                CaptureAuthorization::Denied(D::ConsentAbsent),
            ),
            (
                P::Granted,
                ConsentState::Denied,
                CaptureAuthorization::Denied(D::ConsentDenied),
            ),
            (
                P::Granted,
                ConsentState::Revoked { at: NOW },
                CaptureAuthorization::Denied(D::ConsentRevoked),
            ),
            (
                P::Denied,
                fresh,
                CaptureAuthorization::Denied(D::PermissionDenied),
            ),
            (
                P::NotDetermined,
                fresh,
                CaptureAuthorization::Denied(D::PermissionNotDetermined),
            ),
            (
                P::Restricted,
                fresh,
                CaptureAuthorization::Denied(D::PermissionRestricted),
            ),
            (
                P::Unknown,
                fresh,
                CaptureAuthorization::Denied(D::PermissionUnknown),
            ),
            (P::Granted, fresh, CaptureAuthorization::Granted),
            (
                P::NotRequiredByPlatform,
                fresh,
                CaptureAuthorization::Granted,
            ),
            // Consent is checked first.
            (
                P::Denied,
                ConsentState::Unknown,
                CaptureAuthorization::Denied(D::ConsentAbsent),
            ),
        ];
        for (perm, consent, expected) in cases {
            assert_eq!(
                evaluate_capture_authorization(perm, consent, NOW, None),
                expected,
                "{perm:?} {consent:?}"
            );
        }
    }

    #[test]
    fn stale_consent_is_denied_only_when_a_max_age_is_configured() {
        let old = granted(Duration::from_secs(120));
        let max = Some(Duration::from_secs(60));
        assert_eq!(
            evaluate_capture_authorization(PlatformPermission::Granted, old, NOW, max),
            CaptureAuthorization::Denied(AuthorizationDenial::ConsentStale)
        );
        assert!(
            evaluate_capture_authorization(PlatformPermission::Granted, old, NOW, None)
                .is_granted()
        );
        assert!(
            evaluate_capture_authorization(
                PlatformPermission::Granted,
                granted(Duration::from_secs(59)),
                NOW,
                max
            )
            .is_granted()
        );
        // A grant timestamped in the future (wall-clock skew) has age 0.
        let future = ConsentState::Granted {
            at: WallTime::from_unix_nanos(NOW.unix_nanos() + 10),
        };
        assert!(
            evaluate_capture_authorization(PlatformPermission::Granted, future, NOW, max)
                .is_granted()
        );
    }
}
