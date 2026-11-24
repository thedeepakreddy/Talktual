//! Domain profiles (Phase 5): **data, not code**.
//!
//! A profile adds terminology (canonical terms, aliases and categories) and
//! context hints for one domain. It is versioned, validated, bounded and
//! fingerprinted, and its identity travels with every observation it
//! influenced.
//!
//! **A profile never manufactures certainty.** A profile match is evidence
//! of kind `Profile`; the adapter decides the certainty from the surface
//! form (an exact canonical match is at most `High`, an alias at most
//! `Medium`, an alias shared by several terms is an ambiguous `Low`
//! hypothesis). A profile cannot raise the certainty of other
//! observations, cannot suppress contrary evidence and cannot change the
//! transcript.

use serde::{Deserialize, Serialize};

use crate::error::CeError;
use crate::speech::config::fnv;

use super::model::ProfileRef;

/// Profile schema version.
pub const PROFILE_SCHEMA_VERSION: u32 = 1;

/// Limits that keep a profile bounded.
pub mod limits {
    /// Most terms.
    pub const MAX_TERMS: usize = 2_000;
    /// Most aliases per term.
    pub const MAX_ALIASES: usize = 16;
    /// Longest term or alias, characters.
    pub const MAX_TERM_CHARS: usize = 64;
    /// Most words in a term or alias.
    pub const MAX_TERM_WORDS: usize = 6;
    /// Most categories.
    pub const MAX_CATEGORIES: usize = 64;
    /// Most context hints.
    pub const MAX_HINTS: usize = 256;
}

/// One term.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileTerm {
    /// Canonical form (what the normalized value says).
    pub canonical: String,
    /// Category (one of the profile's `categories`).
    pub category: String,
    /// Other surface forms (e.g. spoken or ASR-typical spellings).
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// A context hint: words that, when spoken, make a profile category a
/// compatible antecedent for a referring expression (e.g. "it" may refer to
/// a `device`). Hints only *widen candidate compatibility*; they never pick
/// a candidate.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextHint {
    /// Profile category.
    pub category: String,
    /// Referring expressions that may refer to it (`it`, `they`, `there`, …).
    pub referents: Vec<String>,
}

/// A domain profile.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainProfile {
    /// Schema version (must be [`PROFILE_SCHEMA_VERSION`]).
    pub schema_version: u32,
    /// Id (`[a-z0-9._-]`, 1..=64).
    pub id: String,
    /// Version (`MAJOR.MINOR.PATCH`).
    pub version: String,
    /// Primary language subtag the terms are in (e.g. `en`).
    pub language: String,
    /// Categories this profile defines (`[a-z_]`, emitted as
    /// `profile:<category>`).
    pub categories: Vec<String>,
    /// Terms.
    pub terms: Vec<ProfileTerm>,
    /// Context hints.
    #[serde(default)]
    pub hints: Vec<ContextHint>,
    /// Id of the evaluation set this profile was evaluated on, if any
    /// (evaluation identity).
    #[serde(default)]
    pub evaluation: Option<String>,
}

fn bad(field: &'static str, allowed: &'static str) -> CeError {
    CeError::invalid_config("config.profile_invalid", "domain profile is invalid")
        .with_context("field", field)
        .with_context("allowed", allowed)
}

/// Fold a surface form for matching: lower case, letters/digits/apostrophes
/// kept, other characters become single spaces. Matching is on folded word
/// sequences, so ASR casing and punctuation do not matter.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = true;
    for c in s.chars() {
        if c.is_alphanumeric() || c == '\'' {
            out.extend(c.to_lowercase());
            space = false;
        } else if !space {
            out.push(' ');
            space = true;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

fn valid_term(s: &str) -> bool {
    let f = fold(s);
    !f.is_empty()
        && s.chars().count() <= limits::MAX_TERM_CHARS
        && f.split(' ').count() <= limits::MAX_TERM_WORDS
        && !s.chars().any(char::is_control)
}

impl DomainProfile {
    /// Validate shape and bounds.
    pub fn validate(&self) -> Result<(), CeError> {
        if self.schema_version != PROFILE_SCHEMA_VERSION {
            return Err(bad("schema_version", "1"));
        }
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c))
        {
            return Err(bad("id", "1..=64 chars of [a-z0-9._-]"));
        }
        let parts: Vec<&str> = self.version.split('.').collect();
        if parts.len() != 3
            || parts
                .iter()
                .any(|p| p.is_empty() || p.len() > 6 || !p.chars().all(|c| c.is_ascii_digit()))
        {
            return Err(bad("version", "MAJOR.MINOR.PATCH"));
        }
        if self.language.is_empty()
            || self.language.len() > 8
            || !self.language.chars().all(|c| c.is_ascii_lowercase())
        {
            return Err(bad("language", "a lower-case primary language subtag"));
        }
        if self.categories.is_empty() || self.categories.len() > limits::MAX_CATEGORIES {
            return Err(bad("categories", "1..=64 categories"));
        }
        for (i, c) in self.categories.iter().enumerate() {
            if c.is_empty()
                || c.len() > 32
                || !c.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_')
                || self.categories[..i].contains(c)
            {
                return Err(bad("categories", "unique, 1..=32 chars of [a-z_]"));
            }
        }
        if self.terms.is_empty() || self.terms.len() > limits::MAX_TERMS {
            return Err(bad("terms", "1..=2000 terms"));
        }
        let mut seen: Vec<(String, &str)> = Vec::new();
        for t in &self.terms {
            if !self.categories.contains(&t.category) {
                return Err(bad("terms.category", "one of the profile's categories"));
            }
            if t.aliases.len() > limits::MAX_ALIASES {
                return Err(bad("terms.aliases", "at most 16 aliases"));
            }
            for form in std::iter::once(&t.canonical).chain(&t.aliases) {
                if !valid_term(form) {
                    return Err(bad(
                        "terms",
                        "non-empty, at most 64 characters and 6 words, no control characters",
                    ));
                }
                let f = fold(form);
                // The same surface form twice in one category is a mistake;
                // across categories it is allowed and reported as ambiguity.
                if seen.iter().any(|(s, c)| *s == f && *c == t.category) {
                    return Err(bad("terms", "no duplicate surface forms within a category"));
                }
                seen.push((f, &t.category));
            }
        }
        if self.hints.len() > limits::MAX_HINTS {
            return Err(bad("hints", "at most 256 hints"));
        }
        for h in &self.hints {
            if !self.categories.contains(&h.category)
                || h.referents.is_empty()
                || h.referents.len() > 16
                || h.referents.iter().any(|r| !valid_term(r))
            {
                return Err(bad(
                    "hints",
                    "a profile category and 1..=16 valid referring expressions",
                ));
            }
        }
        if let Some(e) = &self.evaluation {
            if e.is_empty() || e.len() > 128 || e.chars().any(char::is_control) {
                return Err(bad("evaluation", "1..=128 characters"));
            }
        }
        Ok(())
    }

    /// Stable fingerprint, 16 hex digits.
    pub fn fingerprint(&self) -> String {
        fnv(&format!("profile.v1|{self:?}"))
    }

    /// Identity for provenance.
    pub fn reference(&self) -> ProfileRef {
        ProfileRef {
            id: self.id.clone(),
            version: self.version.clone(),
            fingerprint: self.fingerprint(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> DomainProfile {
        serde_json::from_str(
            r#"{
              "schema_version": 1, "id": "test.devices", "version": "1.0.0",
              "language": "en", "categories": ["device", "room"],
              "terms": [
                {"canonical": "Thermostat", "category": "device", "aliases": ["thermo stat"]},
                {"canonical": "Kitchen", "category": "room"}
              ],
              "hints": [{"category": "device", "referents": ["it"]}]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn valid_profile_validates_and_fingerprints() {
        let p = profile();
        p.validate().unwrap();
        assert_eq!(p.reference().fingerprint.len(), 16);
        let mut q = p.clone();
        q.version = "1.0.1".into();
        assert_ne!(p.fingerprint(), q.fingerprint());
        assert!(
            serde_json::from_str::<DomainProfile>(
                r#"{"schema_version":1,"id":"x","version":"1.0.0","language":"en","categories":["a"],"terms":[],"code":"rm -rf"}"#
            )
            .is_err(),
            "unknown fields (e.g. embedded logic) are rejected"
        );
    }

    #[test]
    fn invalid_profiles_are_rejected() {
        type Mutation = Box<dyn Fn(&mut DomainProfile)>;
        let cases: Vec<Mutation> = vec![
            Box::new(|p| p.schema_version = 2),
            Box::new(|p| p.id = "Bad Id".into()),
            Box::new(|p| p.version = "1.0".into()),
            Box::new(|p| p.language = "EN".into()),
            Box::new(|p| p.categories.clear()),
            Box::new(|p| p.categories.push("device".into())),
            Box::new(|p| p.terms.clear()),
            Box::new(|p| p.terms[0].category = "missing".into()),
            Box::new(|p| p.terms[0].aliases = vec!["x".into(); 17]),
            Box::new(|p| p.terms[0].aliases.push("a b c d e f g".into())),
            Box::new(|p| p.terms[0].aliases.push("   ".into())),
            Box::new(|p| p.terms[0].aliases.push("THERMOSTAT".into())),
            Box::new(|p| p.hints[0].category = "missing".into()),
            Box::new(|p| p.hints[0].referents.clear()),
            Box::new(|p| p.evaluation = Some(String::new())),
        ];
        for (i, m) in cases.iter().enumerate() {
            let mut p = profile();
            m(&mut p);
            let e = p.validate().expect_err(&format!("case {i}"));
            assert_eq!(e.code(), "config.profile_invalid", "case {i}");
        }
        // The same form in two categories is allowed (reported as ambiguity).
        let mut p = profile();
        p.terms.push(ProfileTerm {
            canonical: "Thermostat".into(),
            category: "room".into(),
            aliases: vec![],
        });
        p.validate().unwrap();
    }

    #[test]
    fn folding_ignores_case_and_punctuation() {
        assert_eq!(fold("  Hello, WORLD!  "), "hello world");
        assert_eq!(fold("it's"), "it's");
        assert_eq!(fold("..."), "");
    }
}
