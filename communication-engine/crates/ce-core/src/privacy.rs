//! Privacy classification, redaction, retention and consent representations.
//!
//! Phase 1 *represents* privacy decisions; it does not persist any sensitive
//! data. See `docs/architecture/privacy.md`.

use std::borrow::Cow;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::BoxFuture;
use crate::error::CeError;
use crate::ids::SessionId;
use crate::time::WallTime;

/// Sensitivity class of a piece of data, ordered from least to most sensitive.
///
/// The ordering is meaningful: a policy that permits a class permits every
/// less sensitive class.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum DataClass {
    /// Safe to publish (static descriptors, versions).
    Public,
    /// Operational metadata: identifiers, states, counters, durations.
    #[default]
    Operational,
    /// Data about a person that is not communication content (language
    /// preference, account or device identifiers).
    Personal,
    /// Communication content: transcripts, translations, messages.
    Content,
    /// Biometric or voice data: raw audio, voice prints, speaker embeddings.
    Biometric,
    /// Credentials and keys. Never visible in any observable output.
    Secret,
}

impl DataClass {
    /// Stable lowercase name.
    pub const fn as_str(self) -> &'static str {
        match self {
            DataClass::Public => "public",
            DataClass::Operational => "operational",
            DataClass::Personal => "personal",
            DataClass::Content => "content",
            DataClass::Biometric => "biometric",
            DataClass::Secret => "secret",
        }
    }
}

/// Decides which data classes may appear in logs, metrics labels, error
/// displays and other observable output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedactionPolicy {
    /// Most sensitive class that may be shown. [`DataClass::Secret`] is never
    /// shown regardless of this value.
    pub max_visible: DataClass,
}

impl RedactionPolicy {
    /// The default policy: operational metadata only.
    pub const DEFAULT: RedactionPolicy = RedactionPolicy {
        max_visible: DataClass::Operational,
    };

    /// Whether data of `class` may be shown.
    pub fn permits(&self, class: DataClass) -> bool {
        class != DataClass::Secret && class <= self.max_visible
    }

    /// Return `value` if permitted, otherwise a redaction marker naming only the
    /// class.
    pub fn apply<'a>(&self, class: DataClass, value: &'a str) -> Cow<'a, str> {
        if self.permits(class) {
            Cow::Borrowed(value)
        } else {
            Cow::Owned(format!("[redacted:{}]", class.as_str()))
        }
    }
}

impl Default for RedactionPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How long data may be kept. There is intentionally no "forever" variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RetentionPolicy {
    /// Kept in memory only for the duration of processing.
    #[default]
    Ephemeral,
    /// Discarded when the owning session closes.
    Session,
    /// Discarded after at most this long.
    Bounded {
        /// Maximum retention, in milliseconds.
        max_ms: u64,
    },
}

impl RetentionPolicy {
    /// Upper bound on retention, if expressible as a duration.
    pub fn max_duration(&self) -> Option<Duration> {
        match self {
            RetentionPolicy::Ephemeral => Some(Duration::ZERO),
            RetentionPolicy::Session => None,
            RetentionPolicy::Bounded { max_ms } => Some(Duration::from_millis(*max_ms)),
        }
    }
}

/// Purposes for which user consent may be required.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentPurpose {
    /// Capturing audio or other input from the user's device.
    Capture,
    /// Sending data off the device for processing.
    RemoteProcessing,
    /// Keeping data beyond ephemeral processing.
    Retention,
    /// Creating, storing or using a model of a person's voice. Reserved: CE
    /// implements no voice enrollment or cloning (see privacy.md, "Voice
    /// governance").
    VoiceProfile,
}

/// Recorded state of consent for one purpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum ConsentState {
    /// Not asked or not known. Must be treated as "not granted".
    #[default]
    Unknown,
    /// Granted at the given time.
    Granted {
        /// When consent was given.
        at: WallTime,
    },
    /// Explicitly declined.
    Denied,
    /// Previously granted, then withdrawn.
    Revoked {
        /// When consent was withdrawn.
        at: WallTime,
    },
}

impl ConsentState {
    /// Only an explicit, unrevoked grant counts as consent.
    pub fn is_granted(&self) -> bool {
        matches!(self, ConsentState::Granted { .. })
    }
}

/// Privacy declarations attached to a capability or provider.
///
/// `None` means "not declared" and must be treated as unknown — never as
/// "no", "false" or "safe".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PrivacyFlags {
    /// Data classes the capability processes.
    pub processes: Option<Vec<DataClass>>,
    /// Consent purposes that must be granted before the capability is used.
    pub requires_consent: Option<Vec<ConsentPurpose>>,
    /// Longest retention the capability applies to processed data.
    pub retention: Option<RetentionPolicy>,
    /// Whether processed data leaves the device.
    pub leaves_device: Option<bool>,
}

/// What a data-erasure request targets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "scope")]
pub enum ErasureScope {
    /// Everything retained for one session.
    Session {
        /// The session.
        session_id: SessionId,
    },
    /// Everything retained about one data subject, identified by an opaque,
    /// host-defined reference (never a raw personal identifier).
    Subject {
        /// Opaque subject reference.
        subject_ref: String,
    },
}

/// Outcome of an erasure request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErasureReport {
    /// Number of retained items removed.
    pub items_erased: u64,
    /// Whether the implementation verified that nothing remains.
    pub verified: bool,
}

/// Contract for components that retain data and must be able to delete it.
///
/// **Groundwork only.** Phase 1 persists no sensitive data, so no production
/// component implements this yet. Any future component that retains
/// `Personal`, `Content` or `Biometric` data (including any voice-profile
/// feature) must implement it before it can be accepted.
pub trait DataErasure: Send + Sync {
    /// Erase all data covered by `scope`.
    fn erase<'a>(
        &'a self,
        scope: &'a ErasureScope,
    ) -> BoxFuture<'a, Result<ErasureReport, CeError>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_are_ordered_by_sensitivity() {
        use DataClass::*;
        let ordered = [Public, Operational, Personal, Content, Biometric, Secret];
        assert!(ordered.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn default_policy_only_shows_operational_data() {
        let policy = RedactionPolicy::default();
        assert!(policy.permits(DataClass::Public));
        assert!(policy.permits(DataClass::Operational));
        assert!(!policy.permits(DataClass::Personal));
        assert_eq!(
            policy.apply(DataClass::Content, "hello"),
            "[redacted:content]"
        );
        assert_eq!(
            policy.apply(DataClass::Operational, "state=running"),
            "state=running"
        );
    }

    #[test]
    fn secrets_are_never_visible_even_under_the_most_permissive_policy() {
        let policy = RedactionPolicy {
            max_visible: DataClass::Secret,
        };
        assert!(policy.permits(DataClass::Biometric));
        assert!(!policy.permits(DataClass::Secret));
        assert_eq!(
            policy.apply(DataClass::Secret, "sk-123"),
            "[redacted:secret]"
        );
    }

    #[test]
    fn only_explicit_grants_count_as_consent() {
        assert!(!ConsentState::Unknown.is_granted());
        assert!(!ConsentState::Denied.is_granted());
        assert!(
            !ConsentState::Revoked {
                at: WallTime::from_unix_nanos(1)
            }
            .is_granted()
        );
        assert!(
            ConsentState::Granted {
                at: WallTime::from_unix_nanos(1)
            }
            .is_granted()
        );
    }

    #[test]
    fn privacy_flags_default_to_unknown_and_serialize_that_way() {
        let flags = PrivacyFlags::default();
        assert!(flags.processes.is_none() && flags.leaves_device.is_none());
        let json = serde_json::to_value(&flags).unwrap();
        assert_eq!(json["leaves_device"], serde_json::Value::Null);
        assert_eq!(
            RetentionPolicy::Bounded { max_ms: 5 }.max_duration(),
            Some(Duration::from_millis(5))
        );
        assert_eq!(RetentionPolicy::Session.max_duration(), None);
    }
}
