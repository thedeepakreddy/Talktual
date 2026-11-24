//! Semantic configuration (Phase 5): typed, validated, bounded and
//! fingerprinted. Context is **ephemeral by default**: nothing outlives the
//! stream's semantic session, and remote processing is off unless allowed.

use serde::{Deserialize, Serialize};

use crate::error::CeError;
use crate::speech::config::fnv;

use super::model::{Certainty, Locality};
use super::profile::DomainProfile;

/// Which context a session keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContextScopeMode {
    /// No context: every utterance is interpreted alone (references stay
    /// unresolved).
    Off,
    /// Context lives within one utterance and is dropped at its end.
    Utterance,
    /// Context lives for the stream's semantic session (cleared at stop,
    /// reset, cancellation and shutdown). Longer scopes (user memory) are
    /// not supported.
    #[default]
    Session,
}

/// Context bounds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ContextConfig {
    /// Scope. Default `Session`.
    pub scope: ContextScopeMode,
    /// Most items kept (oldest evicted first). Default 32.
    pub max_items: u32,
    /// Most bytes of mentions and values kept. Default 8192.
    pub max_bytes: u32,
    /// Item lifetime on the stream's audio timeline, ms. Default 120 000.
    pub ttl_ms: u64,
    /// Only items from this many most recent utterances are reference
    /// candidates (1 = the current and the previous). Default 2.
    pub max_utterance_distance: u32,
    /// Only items at or above this certainty enter the context. Default
    /// `Medium` (weak hypotheses never become context).
    pub min_certainty: Certainty,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            scope: ContextScopeMode::Session,
            max_items: 32,
            max_bytes: 8_192,
            ttl_ms: 120_000,
            max_utterance_distance: 2,
            min_certainty: Certainty::Medium,
        }
    }
}

/// Where adapters may run. Disclosure: the chosen locality is in every
/// observation's provenance and in `SemanticCapabilities`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LocalityPolicy {
    /// Allow adapters in another local process. Default false.
    pub allow_local_service: bool,
    /// Allow adapters that send transcript text off the device. Default
    /// false; enabling it is a product/privacy decision of the host.
    pub allow_remote: bool,
}

impl Default for LocalityPolicy {
    fn default() -> Self {
        Self {
            allow_local_service: false,
            allow_remote: false,
        }
    }
}

impl LocalityPolicy {
    /// Whether an adapter of this locality may run.
    pub fn permits(&self, locality: Locality) -> bool {
        match locality {
            Locality::InProcess => true,
            Locality::LocalService => self.allow_local_service,
            Locality::Remote => self.allow_remote,
        }
    }
}

/// Bounded execution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SemanticBudget {
    /// Input queue capacity (revisions). When full, provisional revisions
    /// of the same utterance are coalesced; finals and cancellations are
    /// never dropped. Default 64.
    pub queue: u32,
    /// Longest transcript snapshot accepted, characters. Default 4096.
    pub max_input_chars: u32,
    /// Per-input adapter deadline, ms: a later result is discarded and
    /// reported (`DeadlineExceeded`). Default 250.
    pub deadline_ms: u64,
    /// Most observations per input (excess dropped and reported). Default 64.
    pub max_observations: u32,
    /// Consecutive adapter errors before the worker fails. Default 5.
    pub max_consecutive_errors: u32,
    /// No progress with queued input for this long → stalled. Default 5000.
    pub stall_timeout_ms: u64,
    /// Process provisional (partial) revisions; when false only finals are
    /// interpreted. Default true.
    pub partials: bool,
}

impl Default for SemanticBudget {
    fn default() -> Self {
        Self {
            queue: 64,
            max_input_chars: 4_096,
            deadline_ms: 250,
            max_observations: 64,
            max_consecutive_errors: 5,
            stall_timeout_ms: 5_000,
            partials: true,
        }
    }
}

/// A civil date for normalizing relative dates (only when the host
/// provides one; never taken from the system clock implicitly).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CivilDate {
    /// Year.
    pub year: i32,
    /// Month 1–12.
    pub month: u8,
    /// Day 1–31.
    pub day: u8,
}

impl CivilDate {
    /// Whether the date exists in the proleptic Gregorian calendar.
    pub fn is_valid(&self) -> bool {
        (1..=12).contains(&self.month)
            && self.day >= 1
            && self.day <= days_in_month(self.year, self.month)
    }

    /// The date `days` later (or earlier when negative).
    pub fn add_days(&self, days: i32) -> CivilDate {
        civil_from_days(days_from_civil(*self) + i64::from(days))
    }

    /// Weekday, 0 = Monday.
    pub fn weekday(&self) -> u8 {
        // 1970-01-01 was a Thursday (3).
        (days_from_civil(*self) + 3).rem_euclid(7) as u8
    }
}

fn days_in_month(y: i32, m: u8) -> u8 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

// Howard Hinnant's days_from_civil / civil_from_days.
fn days_from_civil(d: CivilDate) -> i64 {
    let y = i64::from(d.year) - i64::from(d.month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(d.month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d.day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> CivilDate {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    CivilDate {
        year: (y + i64::from(m <= 2)) as i32,
        month: m as u8,
        day: d as u8,
    }
}

/// Per-stream semantic configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SemanticConfig {
    /// Adapter id (`None`: the engine's default semantic adapter).
    pub adapter: Option<String>,
    /// Extract entities. Default true.
    pub entities: bool,
    /// Emit relations between observations. Default true.
    pub relations: bool,
    /// Resolve references against scoped context. Default true.
    pub references: bool,
    /// Label communicative acts (optional capability). Default true.
    pub acts: bool,
    /// Observations below this certainty are not emitted. Default `Low`
    /// (everything, with its uncertainty).
    pub min_certainty: Certainty,
    /// Context.
    pub context: ContextConfig,
    /// Locality policy.
    pub policy: LocalityPolicy,
    /// Reference date for absolute normalization of relative dates
    /// ("tomorrow"). `None`: relative values stay relative.
    pub reference_date: Option<CivilDate>,
    /// Domain profile (data, validated, fingerprinted).
    pub profile: Option<DomainProfile>,
    /// Budget.
    pub budget: SemanticBudget,
}

impl Default for SemanticConfig {
    fn default() -> Self {
        Self {
            adapter: None,
            entities: true,
            relations: true,
            references: true,
            acts: true,
            min_certainty: Certainty::Low,
            context: ContextConfig::default(),
            policy: LocalityPolicy::default(),
            reference_date: None,
            profile: None,
            budget: SemanticBudget::default(),
        }
    }
}

fn bad(field: &'static str, allowed: &'static str) -> CeError {
    CeError::invalid_config(
        "config.semantic_invalid",
        "semantic configuration value is invalid",
    )
    .with_context("field", field)
    .with_context("allowed", allowed)
}

impl SemanticConfig {
    /// Validate ranges, relationships and the profile.
    pub fn validate(&self) -> Result<(), CeError> {
        if let Some(a) = &self.adapter {
            if a.is_empty()
                || a.len() > 64
                || !a
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            {
                return Err(bad("adapter", "1..=64 chars of [A-Za-z0-9._-]"));
            }
        }
        let c = &self.context;
        if !(1..=1_024).contains(&c.max_items)
            || !(256..=1_048_576).contains(&c.max_bytes)
            || !(1_000..=3_600_000).contains(&c.ttl_ms)
            || c.max_utterance_distance > 64
        {
            return Err(bad(
                "context",
                "max_items 1..=1024, max_bytes 256..=1048576, ttl_ms 1000..=3600000, max_utterance_distance <= 64",
            ));
        }
        let b = &self.budget;
        if !(2..=10_000).contains(&b.queue)
            || !(16..=65_536).contains(&b.max_input_chars)
            || !(1..=60_000).contains(&b.deadline_ms)
            || !(1..=4_096).contains(&b.max_observations)
            || b.max_consecutive_errors == 0
            || !(100..=600_000).contains(&b.stall_timeout_ms)
        {
            return Err(bad(
                "budget",
                "queue 2..=10000, max_input_chars 16..=65536, deadline_ms 1..=60000, max_observations 1..=4096, errors >= 1, stall 100..=600000 ms",
            ));
        }
        if self.reference_date.is_some_and(|d| !d.is_valid()) {
            return Err(bad("reference_date", "a valid calendar date"));
        }
        // `references` with `ContextScopeMode::Off` is valid: references are
        // then reported unresolved (`context_disabled`).
        if let Some(p) = &self.profile {
            p.validate()?;
        }
        Ok(())
    }

    /// A stable 64-bit fingerprint (FNV-1a over the canonical debug form,
    /// stable within a CE release), 16 hex digits. Includes the profile.
    pub fn fingerprint(&self) -> String {
        fnv(&format!("semantic.v1|{self:?}"))
    }
}

/// Engine-wide semantic settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SemanticSettings {
    /// Default per-stream configuration.
    pub semantic: SemanticConfig,
    /// Concurrent semantic workers across the engine (1..=64). Default 4.
    pub max_concurrent: u32,
}

impl Default for SemanticSettings {
    fn default() -> Self {
        Self {
            semantic: SemanticConfig::default(),
            max_concurrent: 4,
        }
    }
}

impl SemanticSettings {
    /// Validate.
    pub fn validate(&self) -> Result<(), CeError> {
        if !(1..=64).contains(&self.max_concurrent) {
            return Err(bad("max_concurrent", "1..=64"));
        }
        self.semantic.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate_round_trip_and_are_private() {
        let c = SemanticConfig::default();
        c.validate().unwrap();
        SemanticSettings::default().validate().unwrap();
        assert!(!c.policy.allow_remote && !c.policy.allow_local_service);
        assert!(c.policy.permits(Locality::InProcess));
        assert!(!c.policy.permits(Locality::Remote));
        assert_eq!(c.context.scope, ContextScopeMode::Session);
        let json = serde_json::to_string(&c).unwrap();
        let back: SemanticConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.fingerprint(), c.fingerprint());
        assert!(serde_json::from_str::<SemanticConfig>(r#"{"bogus":1}"#).is_err());
    }

    #[test]
    fn invalid_values_are_rejected_with_one_code() {
        type Mutation = Box<dyn Fn(&mut SemanticConfig)>;
        let cases: Vec<Mutation> = vec![
            Box::new(|c| c.adapter = Some(String::new())),
            Box::new(|c| c.adapter = Some("bad id!".into())),
            Box::new(|c| c.context.max_items = 0),
            Box::new(|c| c.context.max_bytes = 10),
            Box::new(|c| c.context.ttl_ms = 10),
            Box::new(|c| c.context.max_utterance_distance = 1_000),
            Box::new(|c| c.budget.queue = 1),
            Box::new(|c| c.budget.max_input_chars = 1),
            Box::new(|c| c.budget.deadline_ms = 0),
            Box::new(|c| c.budget.max_observations = 0),
            Box::new(|c| c.budget.max_consecutive_errors = 0),
            Box::new(|c| c.budget.stall_timeout_ms = 1),
            Box::new(|c| {
                c.reference_date = Some(CivilDate {
                    year: 2026,
                    month: 2,
                    day: 30,
                })
            }),
        ];
        for (i, m) in cases.iter().enumerate() {
            let mut c = SemanticConfig::default();
            m(&mut c);
            let e = c.validate().expect_err(&format!("case {i}"));
            assert_eq!(e.code(), "config.semantic_invalid", "case {i}");
        }
        let s = SemanticSettings {
            max_concurrent: 0,
            ..SemanticSettings::default()
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn fingerprints_change_with_configuration() {
        let a = SemanticConfig::default();
        let mut b = a.clone();
        b.acts = false;
        assert_ne!(a.fingerprint(), b.fingerprint());
        assert_eq!(a.fingerprint().len(), 16);
    }

    #[test]
    fn civil_date_arithmetic() {
        let d = CivilDate {
            year: 2026,
            month: 11,
            day: 24,
        };
        assert!(d.is_valid());
        assert_eq!(d.weekday(), 1); // Tuesday
        assert_eq!(
            d.add_days(7),
            CivilDate {
                year: 2026,
                month: 12,
                day: 1
            }
        );
        assert_eq!(
            d.add_days(38),
            CivilDate {
                year: 2027,
                month: 1,
                day: 1
            }
        );
        let leap = CivilDate {
            year: 2028,
            month: 2,
            day: 28,
        };
        assert_eq!(leap.add_days(1).day, 29);
        assert_eq!(d.add_days(-24).month, 10);
        assert!(
            !CivilDate {
                year: 2027,
                month: 2,
                day: 29
            }
            .is_valid()
        );
    }
}
