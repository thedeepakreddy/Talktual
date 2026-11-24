//! Session scaffold.
//!
//! A session is the unit of correlation for one communication context. In
//! Phase 1 it carries identity, state, configuration and correlation only —
//! no audio, speech or text flows through it.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::capability::LanguageTag;
use crate::error::CeError;
use crate::ids::{CorrelationId, ModeId, SessionId};
use crate::privacy::{ConsentPurpose, ConsentState, RetentionPolicy};
use crate::time::WallTime;

/// Session lifecycle.
///
/// ```text
/// Created ─► Active ─► Closing ─► Closed
///    │          │          │
///    └──────────┴──────────┴──► Failed ─► Closed
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Allocated, not yet active.
    Created,
    /// Active.
    Active,
    /// Closing.
    Closing,
    /// Closed (terminal).
    Closed,
    /// Failed; must be closed.
    Failed,
}

impl SessionState {
    /// All states.
    pub const ALL: [SessionState; 5] = [
        SessionState::Created,
        SessionState::Active,
        SessionState::Closing,
        SessionState::Closed,
        SessionState::Failed,
    ];

    /// The transition table.
    pub const fn can_transition_to(self, next: SessionState) -> bool {
        use SessionState::*;
        matches!(
            (self, next),
            (Created, Active)
                | (Created, Closing)
                | (Created, Failed)
                | (Active, Closing)
                | (Active, Failed)
                | (Closing, Closed)
                | (Closing, Failed)
                | (Failed, Closed)
        )
    }

    /// Validate a transition.
    pub fn transition_to(self, next: SessionState) -> Result<SessionState, CeError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(CeError::lifecycle(
                "lifecycle.illegal_session_transition",
                "illegal session state transition",
            )
            .with_context("from", self.to_string())
            .with_context("to", next.to_string()))
        }
    }

    /// Stable lowercase name.
    pub const fn as_str(self) -> &'static str {
        match self {
            SessionState::Created => "created",
            SessionState::Active => "active",
            SessionState::Closing => "closing",
            SessionState::Closed => "closed",
            SessionState::Failed => "failed",
        }
    }
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Optional language metadata for a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SessionLanguages {
    /// Expected input language, if known.
    pub input: Option<LanguageTag>,
    /// Desired output languages.
    #[serde(default)]
    pub outputs: Vec<LanguageTag>,
}

/// Session configuration. Everything is optional metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfig {
    /// Language metadata (classified `Personal`; never logged by default).
    pub languages: Option<SessionLanguages>,
    /// Consumer-defined mode identifier. CE attaches no meaning to it.
    pub mode: Option<ModeId>,
    /// Retention applied to data produced in the session.
    pub retention: RetentionPolicy,
    /// Consent recorded for this session, per purpose. Missing = unknown.
    pub consents: BTreeMap<ConsentPurpose, ConsentState>,
}

/// Maximum output languages per session.
pub const MAX_OUTPUT_LANGUAGES: usize = 16;

impl SessionConfig {
    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), CeError> {
        if let Some(langs) = &self.languages {
            if langs.outputs.len() > MAX_OUTPUT_LANGUAGES {
                return Err(CeError::invalid_input(
                    "session.too_many_languages",
                    "too many output languages",
                )
                .with_context("max", MAX_OUTPUT_LANGUAGES.to_string()));
            }
            let mut seen = std::collections::BTreeSet::new();
            if !langs.outputs.iter().all(|l| seen.insert(l)) {
                return Err(CeError::invalid_input(
                    "session.duplicate_language",
                    "output languages must be unique",
                ));
            }
        }
        Ok(())
    }

    /// Consent state for `purpose` (unknown if not recorded).
    pub fn consent(&self, purpose: ConsentPurpose) -> ConsentState {
        self.consents.get(&purpose).copied().unwrap_or_default()
    }
}

/// Snapshot of a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Identity.
    pub id: SessionId,
    /// Correlation id shared by everything the session causes.
    pub correlation_id: CorrelationId,
    /// State.
    pub state: SessionState,
    /// Configuration.
    pub config: SessionConfig,
    /// Creation time.
    pub created_at: WallTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_transition_table() {
        use SessionState::*;
        let legal = [
            (Created, Active),
            (Created, Closing),
            (Created, Failed),
            (Active, Closing),
            (Active, Failed),
            (Closing, Closed),
            (Closing, Failed),
            (Failed, Closed),
        ];
        for from in SessionState::ALL {
            for to in SessionState::ALL {
                assert_eq!(
                    from.can_transition_to(to),
                    legal.contains(&(from, to)),
                    "{from}->{to}"
                );
                assert_eq!(from.transition_to(to).is_ok(), legal.contains(&(from, to)));
            }
        }
    }

    #[test]
    fn config_validation_and_consent_defaults() {
        let tag = |t: &str| LanguageTag::new(t).unwrap();
        let mut cfg = SessionConfig {
            languages: Some(SessionLanguages {
                input: Some(tag("hu")),
                outputs: vec![tag("en"), tag("de")],
            }),
            ..SessionConfig::default()
        };
        cfg.validate().unwrap();
        assert_eq!(cfg.consent(ConsentPurpose::Capture), ConsentState::Unknown);
        cfg.languages.as_mut().unwrap().outputs.push(tag("en"));
        assert_eq!(
            cfg.validate().unwrap_err().code(),
            "session.duplicate_language"
        );
        cfg.languages.as_mut().unwrap().outputs = (0..17).map(|_| tag("en")).collect();
        assert_eq!(
            cfg.validate().unwrap_err().code(),
            "session.too_many_languages"
        );
        let parsed: SessionConfig = serde_json::from_str(
            r#"{"mode":"example.push_to_talk","consents":{"capture":{"state":"denied"}}}"#,
        )
        .unwrap();
        assert_eq!(
            parsed.consent(ConsentPurpose::Capture),
            ConsentState::Denied
        );
        assert!(serde_json::from_str::<SessionConfig>(r#"{"transcript":"x"}"#).is_err());
    }
}
