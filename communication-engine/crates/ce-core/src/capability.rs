//! Capability descriptors and requirements.
//!
//! A capability is something a module offers (e.g. `ce.audio.capture`) with a
//! version. Modules depend on *capabilities*, not on other modules, so a
//! provider can be swapped without changing its dependents.
//!
//! **Unknown stays unknown:** every descriptive field is optional and `None`
//! means "not declared". Consumers must not read `None` as `false`, "none" or
//! "safe".

use serde::{Deserialize, Serialize};

use crate::error::{CeError, ErrorCategory};
use crate::ids::{CapabilityId, ModuleId};
use crate::privacy::PrivacyFlags;
use crate::resource::ResourceHints;
use crate::version::{Version, VersionReq};

/// Kind of data flowing into or out of a capability. This is also how a
/// capability's modality is expressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    /// Audio samples or encoded audio.
    Audio,
    /// Text.
    Text,
    /// Video frames.
    Video,
    /// Still images.
    Image,
    /// Structured data (records, annotations).
    Structured,
    /// Control signals (start/stop, turn-taking).
    Control,
}

/// Where a capability executes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLocation {
    /// On the user's device.
    OnDevice,
    /// On a host the user controls on the local network.
    LocalNetwork,
    /// On a remote service.
    Remote,
}

/// Whether more than one module may provide a capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    /// Any number of providers.
    #[default]
    Shared,
    /// At most one provider per runtime; a second registration is a conflict.
    Exclusive,
}

/// Language coverage of a capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "languages")]
pub enum LanguageSupport {
    /// Language-independent.
    Any,
    /// Exactly these languages.
    Listed(Vec<LanguageTag>),
}

/// A syntactically valid BCP 47 language tag (e.g. `en`, `hu-HU`, `zh-Hant`).
///
/// Only syntax is checked (subtags of 1–8 ASCII alphanumerics, primary subtag
/// 2–3 letters or `und`/`x`); registry membership is not.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LanguageTag(String);

impl LanguageTag {
    /// Validate a tag.
    pub fn new(tag: impl Into<String>) -> Result<Self, CeError> {
        let tag = tag.into();
        let invalid = || {
            CeError::invalid_input("language.invalid_tag", "invalid BCP 47 language tag")
                .with_context("tag", tag.clone())
        };
        if tag.is_empty() || tag.len() > 35 {
            return Err(invalid());
        }
        let mut parts = tag.split('-');
        let primary = parts.next().unwrap_or_default();
        let primary_ok = (2..=3).contains(&primary.len())
            && primary.chars().all(|c| c.is_ascii_alphabetic())
            || primary.eq_ignore_ascii_case("x");
        if !primary_ok {
            return Err(invalid());
        }
        for part in parts {
            if part.is_empty() || part.len() > 8 || !part.chars().all(|c| c.is_ascii_alphanumeric())
            {
                return Err(invalid());
            }
        }
        Ok(Self(tag))
    }

    /// The tag as given.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for LanguageTag {
    type Error = CeError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<LanguageTag> for String {
    fn from(value: LanguageTag) -> Self {
        value.0
    }
}

/// Describes one capability a module provides.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDescriptor {
    /// Stable capability identity.
    pub id: CapabilityId,
    /// Version of the capability's interface.
    pub version: Version,
    /// Whether other modules may provide the same capability.
    #[serde(default)]
    pub cardinality: Cardinality,
    /// Accepted inputs (`None` = unknown).
    pub inputs: Option<Vec<MediaKind>>,
    /// Produced outputs (`None` = unknown).
    pub outputs: Option<Vec<MediaKind>>,
    /// Whether the capability works incrementally on streams.
    pub streaming: Option<bool>,
    /// Language coverage.
    pub languages: Option<LanguageSupport>,
    /// Where it executes.
    pub execution: Option<ExecutionLocation>,
    /// Resource hints.
    #[serde(default)]
    pub resources: ResourceHints,
    /// Privacy declarations, including consent requirements.
    #[serde(default)]
    pub privacy: PrivacyFlags,
}

impl CapabilityDescriptor {
    /// A descriptor with every optional property unknown.
    pub fn new(id: CapabilityId, version: Version) -> Self {
        Self {
            id,
            version,
            cardinality: Cardinality::Shared,
            inputs: None,
            outputs: None,
            streaming: None,
            languages: None,
            execution: None,
            resources: ResourceHints::default(),
            privacy: PrivacyFlags::default(),
        }
    }

    /// Set cardinality.
    pub fn cardinality(mut self, cardinality: Cardinality) -> Self {
        self.cardinality = cardinality;
        self
    }

    /// Declare inputs and outputs.
    pub fn io(mut self, inputs: Vec<MediaKind>, outputs: Vec<MediaKind>) -> Self {
        self.inputs = Some(inputs);
        self.outputs = Some(outputs);
        self
    }

    /// Declare streaming support.
    pub fn streaming(mut self, streaming: bool) -> Self {
        self.streaming = Some(streaming);
        self
    }

    /// Declare language support.
    pub fn languages(mut self, languages: LanguageSupport) -> Self {
        self.languages = Some(languages);
        self
    }

    /// Declare execution location.
    pub fn execution(mut self, execution: ExecutionLocation) -> Self {
        self.execution = Some(execution);
        self
    }

    /// Declare privacy flags.
    pub fn privacy(mut self, privacy: PrivacyFlags) -> Self {
        self.privacy = privacy;
        self
    }

    /// Declare resource hints.
    pub fn resources(mut self, resources: ResourceHints) -> Self {
        self.resources = resources;
        self
    }
}

/// A module's dependency on a capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequirement {
    /// Required capability.
    pub capability: CapabilityId,
    /// Acceptable versions.
    pub version: VersionReq,
    /// If true, the module starts even when no provider exists.
    #[serde(default)]
    pub optional: bool,
}

impl CapabilityRequirement {
    /// A mandatory requirement.
    pub fn required(capability: CapabilityId, version: VersionReq) -> Self {
        Self {
            capability,
            version,
            optional: false,
        }
    }

    /// An optional requirement.
    pub fn optional(capability: CapabilityId, version: VersionReq) -> Self {
        Self {
            capability,
            version,
            optional: true,
        }
    }

    /// Whether `descriptor` satisfies this requirement.
    pub fn is_satisfied_by(&self, descriptor: &CapabilityDescriptor) -> bool {
        descriptor.id == self.capability && self.version.matches(&descriptor.version)
    }
}

/// A capability as exposed by the registry: the descriptor plus who provides it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityInfo {
    /// The descriptor.
    pub descriptor: CapabilityDescriptor,
    /// The providing module.
    pub provider: ModuleId,
    /// The provider's current lifecycle state.
    pub provider_state: crate::lifecycle::ModuleState,
}

/// Error for a capability that cannot be resolved.
pub(crate) fn resolution_error(code: &'static str, message: &'static str) -> CeError {
    CeError::new(ErrorCategory::CapabilityResolution, code, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_tags_are_syntax_checked() {
        for ok in ["en", "hu-HU", "zh-Hant-TW", "und", "x-private", "es-419"] {
            assert!(LanguageTag::new(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "e",
            "english",
            "en_US",
            "en-",
            "en--US",
            "en-toolongsubtag",
            "1a",
        ] {
            assert!(LanguageTag::new(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn requirements_match_on_id_and_version() {
        let id = CapabilityId::new("ce.test.cap").unwrap();
        let desc = CapabilityDescriptor::new(id.clone(), Version::new(1, 4, 0));
        assert!(
            CapabilityRequirement::required(id.clone(), VersionReq::parse("^1.2").unwrap())
                .is_satisfied_by(&desc)
        );
        assert!(
            !CapabilityRequirement::required(id, VersionReq::parse("^2").unwrap())
                .is_satisfied_by(&desc)
        );
        let other = CapabilityId::new("ce.test.other").unwrap();
        assert!(!CapabilityRequirement::required(other, VersionReq::STAR).is_satisfied_by(&desc));
    }

    #[test]
    fn descriptors_keep_unknown_fields_unknown_through_serde() {
        let desc = CapabilityDescriptor::new(
            CapabilityId::new("ce.test.cap").unwrap(),
            Version::new(1, 0, 0),
        )
        .streaming(true);
        let json = serde_json::to_string(&desc).unwrap();
        let back: CapabilityDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(back, desc);
        assert_eq!(back.execution, None);
        assert_eq!(back.privacy.leaves_device, None);
        assert_eq!(back.streaming, Some(true));
        // Minimal input: only id and version; everything else unknown.
        let minimal: CapabilityDescriptor = serde_json::from_str(
            r#"{"id":"ce.test.cap","version":"1.0.0","inputs":null,"outputs":null,
                "streaming":null,"languages":null,"execution":null}"#,
        )
        .unwrap();
        assert_eq!(minimal.cardinality, Cardinality::Shared);
        assert!(
            serde_json::from_str::<CapabilityDescriptor>(
                r#"{"id":"ce.test.cap","version":"1.0.0","inputs":null,"outputs":null,
                "streaming":null,"languages":null,"execution":null,"bogus":1}"#
            )
            .is_err()
        );
    }
}
