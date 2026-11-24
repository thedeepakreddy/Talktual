//! `ce.semantic.rules` — the deterministic English rule adapter.
//!
//! In process, no model, no network, no dependency. Designed for
//! recognizer output (upper case, no punctuation) and for written text.
//!
//! **Entities** (domain-neutral built-ins; see `docs/architecture/semantic.md`):
//!
//! | Category | Example | Normalized |
//! |---|---|---|
//! | `number` | "twenty five", "2.5" | integer / decimal |
//! | `ordinal` | "third", "21st" | ordinal |
//! | `quantity` | "three kilometers", "two boxes" | amount + unit (as spoken) |
//! | `money` | "five dollars and fifty cents" | amount + ISO 4217 code |
//! | `percentage` | "ten percent" | value |
//! | `time_of_day` | "at nine thirty", "seven p m", "noon" | hour, minute, am/pm only if said |
//! | `relative_day` | "tomorrow" | day offset; a date only with a configured reference date |
//! | `weekday` | "next monday" | weekday; never resolved to a date (ambiguous) |
//! | `date` | "march third twenty twenty six" | stated parts only |
//! | `duration` | "an hour and a half" | seconds |
//! | `digit_sequence` | "four two seven one" | the digits (codes, numbers; not interpreted) |
//! | `profile:<category>` | profile terms | canonical form |
//!
//! **Not covered**: person, organization and location names (need an ML
//! model, deferred); other languages (inputs with a non-English language
//! tag produce no observations).
//!
//! **References**: pronouns and demonstratives are resolved only to
//! *compatible* context items the processor permits; exactly one candidate
//! → resolved, several → ambiguous (candidates listed), none → unresolved.
//!
//! **Acts** (taxonomy `ce.acts` 1.0.0, optional): question, request,
//! confirmation, refusal, acknowledgement, correction, statement — or an
//! abstention (ambiguous cues, no cue, out of taxonomy such as greetings).
//! Labels are cue-based hypotheses, never facts.
//!
//! Certainty is a rule tier (not a probability) and never calibrated.

use std::collections::BTreeSet;

use ce_core::CeError;
use ce_core::semantic::profile::fold;
use ce_core::semantic::{
    ActAbstention, ActDecision, Attribute, Certainty, CivilDate, CommunicativeAct, ContextItem,
    DomainProfile, Entity, Epistemic, Evidence, EvidenceKind, Locality, NormalizedValue,
    ReferenceCandidate, ReferenceKind, ReferenceResolution, Resolution, SemanticCapabilities,
    SemanticConfig, SemanticInput, TextSpan, Uncertainty,
};

use crate::adapter::{
    ContextView, Draft, DraftPayload, RelationTarget, SemanticAdapter, SemanticFactory,
};
use crate::text::{Tok, digits, number, ordinal, single_digit, tokenize, unit};

/// Adapter id.
pub const RULES_ADAPTER_ID: &str = "ce.semantic.rules";
/// Adapter version (bump when any rule changes; it is in provenance).
pub const RULES_VERSION: &str = "1.0.0";
/// Act taxonomy id.
pub const ACT_TAXONOMY: &str = "ce.acts";
/// Act taxonomy version.
pub const ACT_TAXONOMY_VERSION: &str = "1.0.0";
/// Act labels.
pub const ACT_LABELS: [&str; 7] = [
    "question",
    "request",
    "confirmation",
    "refusal",
    "acknowledgement",
    "correction",
    "statement",
];
/// Built-in entity categories.
pub const CATEGORIES: [&str; 11] = [
    "number",
    "ordinal",
    "quantity",
    "money",
    "percentage",
    "time_of_day",
    "relative_day",
    "weekday",
    "date",
    "duration",
    "digit_sequence",
];

/// Factory for [`RulesAdapter`].
#[derive(Clone, Copy, Debug, Default)]
pub struct RulesFactory;

impl RulesFactory {
    /// Static capabilities.
    pub fn describe() -> SemanticCapabilities {
        SemanticCapabilities {
            adapter: RULES_ADAPTER_ID.into(),
            version: RULES_VERSION.into(),
            locality: Locality::InProcess,
            deterministic: true,
            entities: true,
            relations: true,
            references: true,
            acts: true,
            categories: CATEGORIES.iter().map(|s| (*s).to_string()).collect(),
            normalizes: CATEGORIES.iter().map(|s| (*s).to_string()).collect(),
            act_taxonomies: vec![format!("{ACT_TAXONOMY}@{ACT_TAXONOMY_VERSION}")],
            languages: vec!["en".into()],
            score_semantics: None,
            requires_load: false,
        }
    }
}

impl SemanticFactory for RulesFactory {
    fn capabilities(&self) -> SemanticCapabilities {
        Self::describe()
    }

    fn create(&self, config: &SemanticConfig) -> Result<Box<dyn SemanticAdapter>, CeError> {
        Ok(Box::new(RulesAdapter::new(config)?))
    }
}

#[derive(Debug)]
struct ProfileEntry {
    words: Vec<String>,
    term: usize,
    canonical: bool,
}

#[derive(Debug)]
struct CompiledProfile {
    profile: DomainProfile,
    entries: Vec<ProfileEntry>,
    max_words: usize,
}

impl CompiledProfile {
    fn new(profile: &DomainProfile) -> Self {
        let mut entries = Vec::new();
        for (i, t) in profile.terms.iter().enumerate() {
            entries.push(ProfileEntry {
                words: fold(&t.canonical).split(' ').map(str::to_string).collect(),
                term: i,
                canonical: true,
            });
            for a in &t.aliases {
                entries.push(ProfileEntry {
                    words: fold(a).split(' ').map(str::to_string).collect(),
                    term: i,
                    canonical: false,
                });
            }
        }
        let max_words = entries.iter().map(|e| e.words.len()).max().unwrap_or(1);
        Self {
            profile: profile.clone(),
            entries,
            max_words,
        }
    }

    fn hinted(&self, referent: &str) -> Vec<String> {
        self.profile
            .hints
            .iter()
            .filter(|h| h.referents.iter().any(|r| fold(r) == referent))
            .map(|h| format!("profile:{}", h.category))
            .collect()
    }
}

/// The rule adapter.
#[derive(Debug)]
pub struct RulesAdapter {
    entities: bool,
    relations: bool,
    references: bool,
    acts: bool,
    act_labels: Option<Vec<String>>,
    reference_date: Option<CivilDate>,
    profile: Option<CompiledProfile>,
}

impl RulesAdapter {
    /// An adapter for this configuration.
    pub fn new(config: &SemanticConfig) -> Result<Self, CeError> {
        if let Some(p) = &config.profile {
            p.validate()?;
            if p.language != "en" {
                return Err(CeError::invalid_config(
                    "semantic.profile_language_unsupported",
                    "the rule adapter supports English profiles only",
                ));
            }
        }
        if let Some(l) = &config.act_labels {
            if let Some(bad) = l.iter().find(|x| !ACT_LABELS.contains(&x.as_str())) {
                return Err(CeError::new(
                    ce_core::ErrorCategory::CapabilityResolution,
                    "semantic.act_label_unsupported",
                    "the act taxonomy has no such label",
                )
                .with_context("label", bad.clone()));
            }
        }
        Ok(Self {
            entities: config.entities,
            relations: config.relations,
            references: config.references,
            acts: config.acts,
            act_labels: config.act_labels.clone(),
            reference_date: config.reference_date,
            profile: config.profile.as_ref().map(CompiledProfile::new),
        })
    }
}

struct Found {
    a: usize,
    b: usize,
    category: String,
    normalized: Option<NormalizedValue>,
    level: Certainty,
    epistemic: Epistemic,
    rule: &'static str,
    attributes: Vec<Attribute>,
}

fn attr(name: &str, value: impl Into<String>) -> Attribute {
    Attribute {
        name: name.into(),
        value: value.into(),
    }
}

impl Found {
    fn new(
        a: usize,
        b: usize,
        category: &str,
        normalized: NormalizedValue,
        level: Certainty,
        rule: &'static str,
    ) -> Self {
        Self {
            a,
            b,
            category: category.into(),
            normalized: Some(normalized),
            level,
            epistemic: Epistemic::Observation,
            rule,
            attributes: Vec::new(),
        }
    }
}

fn currency(w: &str) -> Option<(&'static str, Certainty)> {
    Some(match w {
        "dollar" | "dollars" => ("USD", Certainty::High),
        "buck" | "bucks" => ("USD", Certainty::Medium),
        "euro" | "euros" => ("EUR", Certainty::High),
        "rupee" | "rupees" => ("INR", Certainty::High),
        "yen" => ("JPY", Certainty::High),
        _ => return None,
    })
}

fn measure(w: &str) -> Option<&'static str> {
    Some(match w {
        "kilometer" | "kilometers" | "kilometre" | "kilometres" | "km" => "kilometer",
        "mile" | "miles" => "mile",
        "meter" | "meters" | "metre" | "metres" => "meter",
        "centimeter" | "centimeters" | "centimetre" | "centimetres" | "cm" => "centimeter",
        "millimeter" | "millimeters" | "mm" => "millimeter",
        "kilogram" | "kilograms" | "kilo" | "kilos" | "kg" => "kilogram",
        "gram" | "grams" => "gram",
        "liter" | "liters" | "litre" | "litres" => "liter",
        "milliliter" | "milliliters" | "ml" => "milliliter",
        "gallon" | "gallons" => "gallon",
        "foot" | "feet" => "foot",
        "inch" | "inches" => "inch",
        "degree" | "degrees" => "degree",
        "ounce" | "ounces" => "ounce",
        "ton" | "tons" | "tonne" | "tonnes" => "ton",
        _ => return None,
    })
}

fn time_unit(w: &str) -> Option<u64> {
    Some(match w {
        "second" | "seconds" => 1,
        "minute" | "minutes" => 60,
        "hour" | "hours" => 3_600,
        "day" | "days" => 86_400,
        "week" | "weeks" => 604_800,
        _ => return None,
    })
}

fn month(w: &str) -> Option<u8> {
    Some(match w {
        "january" => 1,
        "february" => 2,
        "march" => 3,
        "april" => 4,
        "may" => 5,
        "june" => 6,
        "july" => 7,
        "august" => 8,
        "september" => 9,
        "october" => 10,
        "november" => 11,
        "december" => 12,
        _ => return None,
    })
}

fn weekday(w: &str) -> Option<(u8, bool)> {
    let (base, plural) = match w.strip_suffix('s') {
        Some(b) if b.ends_with("day") => (b, true),
        _ => (w, false),
    };
    let d = match base {
        "monday" => 0,
        "tuesday" => 1,
        "wednesday" => 2,
        "thursday" => 3,
        "friday" => 4,
        "saturday" => 5,
        "sunday" => 6,
        _ => return None,
    };
    Some((d, plural))
}

const STOP_PLURAL: [&str; 24] = [
    "is",
    "was",
    "has",
    "does",
    "this",
    "its",
    "us",
    "yes",
    "his",
    "hers",
    "ours",
    "yours",
    "as",
    "plus",
    "less",
    "minus",
    "always",
    "sometimes",
    "perhaps",
    "thanks",
    "news",
    "series",
    "towards",
    "across",
];

const IRREGULAR_PLURAL: [&str; 6] = ["people", "children", "men", "women", "persons", "geese"];

fn counted_noun(w: &str) -> bool {
    if IRREGULAR_PLURAL.contains(&w) {
        return true;
    }
    w.len() >= 4
        && w.ends_with('s')
        && !w.ends_with("ss")
        && w.chars().all(|c| c.is_ascii_alphabetic())
        && !STOP_PLURAL.contains(&w)
        && time_unit(w).is_none()
        && weekday(w).is_none()
        && month(w).is_none()
        && measure(w).is_none()
        && currency(w).is_none()
}

fn is_unit_word(w: &str) -> bool {
    currency(w).is_some()
        || measure(w).is_some()
        || time_unit(w).is_some()
        || matches!(
            w,
            "percent"
                | "pound"
                | "pounds"
                | "cent"
                | "cents"
                | "point"
                | "of"
                | "times"
                | "more"
                | "less"
        )
        || counted_noun(w)
}

fn year(t: &[Tok], i: usize) -> Option<(i32, usize)> {
    let n = number(t, i)?;
    if n.integer && (1_000.0..=2_999.0).contains(&n.value) {
        return Some((n.value as i32, n.end));
    }
    if n.digits || !n.integer || !(10.0..=29.0).contains(&n.value) || n.end != i + 1 {
        return None;
    }
    let first = n.value as i32;
    let w = t.get(n.end)?.w.as_str();
    if w == "oh" || w == "o" {
        let u = t
            .get(n.end + 1)
            .and_then(|x| unit(&x.w))
            .filter(|u| *u < 10)?;
        return Some((first * 100 + u as i32, n.end + 2));
    }
    let s = number(t, n.end)?;
    if s.digits || !s.integer || !(10.0..=99.0).contains(&s.value) {
        return None;
    }
    Some((first * 100 + s.value as i32, s.end))
}

fn meridiem(t: &[Tok], i: usize) -> Option<(bool, usize)> {
    let w = t.get(i)?.w.as_str();
    let next = t.get(i + 1).map(|x| x.w.as_str());
    match w {
        "am" | "a.m" => Some((false, i + 1)),
        "pm" | "p.m" => Some((true, i + 1)),
        "a" if next == Some("m") => Some((false, i + 2)),
        "p" if next == Some("m") => Some((true, i + 2)),
        "in" if next == Some("the") => match t.get(i + 2).map(|x| x.w.as_str()) {
            Some("morning") => Some((false, i + 3)),
            Some("afternoon" | "evening") => Some((true, i + 3)),
            _ => None,
        },
        "at" if next == Some("night") => Some((true, i + 2)),
        _ => None,
    }
}

fn oclock(t: &[Tok], i: usize) -> Option<usize> {
    match t.get(i).map(|x| x.w.as_str()) {
        Some("o'clock" | "oclock") => Some(i + 1),
        Some("o") if t.get(i + 1).is_some_and(|x| x.w == "clock") => Some(i + 2),
        _ => None,
    }
}

fn minute(t: &[Tok], i: usize) -> Option<(u8, usize)> {
    let w = t.get(i)?.w.as_str();
    if w == "oh" || w == "o" {
        let u = t
            .get(i + 1)
            .and_then(|x| unit(&x.w))
            .filter(|u| (1..10).contains(u))?;
        return Some((u as u8, i + 2));
    }
    let n = number(t, i)?;
    if n.digits || !n.integer || !(1.0..=59.0).contains(&n.value) {
        return None;
    }
    Some((n.value as u8, n.end))
}

const TIME_CUES: [&str; 8] = [
    "at", "by", "until", "till", "around", "before", "after", "from",
];

impl RulesAdapter {
    fn time(&self, t: &[Tok], i: usize) -> Option<Found> {
        let w = t[i].w.as_str();
        let tod = |h: u8, m: u8, pm: Option<bool>| NormalizedValue::TimeOfDay {
            hour: h,
            minute: m,
            pm,
        };
        match w {
            "noon" | "midday" => {
                return Some(Found::new(
                    i,
                    i + 1,
                    "time_of_day",
                    tod(12, 0, Some(true)),
                    Certainty::High,
                    "time.noon",
                ));
            }
            "midnight" => {
                return Some(Found::new(
                    i,
                    i + 1,
                    "time_of_day",
                    tod(0, 0, None),
                    Certainty::High,
                    "time.midnight",
                ));
            }
            "half" | "quarter" => {
                let rel = t.get(i + 1)?.w.as_str();
                if rel != "past" && rel != "to" {
                    return None;
                }
                let h = number(t, i + 2)
                    .filter(|n| n.integer && !n.digits && (1.0..=12.0).contains(&n.value))?;
                let hour = h.value as u8;
                let (hour, min) = match (w, rel) {
                    ("half", "past") => (hour, 30),
                    ("quarter", "past") => (hour, 15),
                    ("quarter", "to") => (if hour == 1 { 12 } else { hour - 1 }, 45),
                    _ => return None,
                };
                let (pm, end) = meridiem(t, h.end).map_or((None, h.end), |(p, e)| (Some(p), e));
                return Some(Found::new(
                    i,
                    end,
                    "time_of_day",
                    tod(hour, min, pm),
                    Certainty::High,
                    "time.past_to",
                ));
            }
            _ => {}
        }
        if let Some((h, m)) = w.split_once(':') {
            let (h, m): (u8, u8) = (h.parse().ok()?, m.parse().ok()?);
            if h > 23 || m > 59 || w.len() > 5 {
                return None;
            }
            let (pm, end) = meridiem(t, i + 1).map_or((None, i + 1), |(p, e)| (Some(p), e));
            return Some(Found::new(
                i,
                end,
                "time_of_day",
                tod(h, m, pm),
                Certainty::High,
                "time.clock_digits",
            ));
        }
        let h = number(t, i)
            .filter(|n| n.integer && n.end == i + 1 && (0.0..=23.0).contains(&n.value))?;
        let hour = h.value as u8;
        let mut end = h.end;
        let mut min = 0u8;
        let mut strong = false;
        if let Some(e) = oclock(t, end) {
            end = e;
            strong = true;
        } else if let Some((m, e)) = minute(t, end) {
            min = m;
            end = e;
        }
        let mut pm = None;
        if let Some((p, e)) = meridiem(t, end) {
            pm = Some(p);
            end = e;
            strong = true;
        }
        let cue = i > 0 && TIME_CUES.contains(&t[i - 1].w.as_str());
        // "EIGHT THIRTY", "FIVE OH FIVE": an hour word 1–12 followed by a
        // spoken minute (tens word or "oh" + digit) reads as a clock time
        // even without a cue word (Medium).
        let clock_pair = !strong
            && !h.digits
            && (1..=12).contains(&hour)
            && end > h.end
            && t.get(h.end)
                .is_some_and(|x| crate::text::tens(&x.w).is_some() || x.w == "oh" || x.w == "o");
        if !(strong || cue || clock_pair) {
            return None;
        }
        if pm.is_some() && !(1..=12).contains(&hour) {
            return None;
        }
        if !strong && (hour == 0 || (hour > 12 && end == h.end)) {
            return None;
        }
        if t.get(end).is_some_and(|x| is_unit_word(&x.w)) {
            return None;
        }
        let mut f = Found::new(
            i,
            end,
            "time_of_day",
            tod(hour, min, pm),
            if strong {
                Certainty::High
            } else {
                Certainty::Medium
            },
            if strong { "time.marked" } else { "time.cued" },
        );
        if pm.is_none() && hour <= 12 {
            f.attributes.push(attr("meridiem", "unspecified"));
        }
        Some(f)
    }

    fn date(&self, t: &[Tok], i: usize) -> Option<Found> {
        let valid = |y: Option<i32>, m: u8, d: u8| {
            CivilDate {
                year: y.unwrap_or(2000),
                month: m,
                day: d,
            }
            .is_valid()
        };
        let date = |year, month: u8, day| NormalizedValue::Date {
            year,
            month: Some(month),
            day,
        };
        if let Some(m) = month(&t[i].w) {
            // month [day] [year]
            let mut end = i + 1;
            let mut day = None;
            let mut level = Certainty::Medium;
            if let Some((d, e)) = ordinal(t, end) {
                day = Some(d as u8);
                end = e;
                level = Certainty::High;
            } else if let Some(n) = number(t, end).filter(|n| {
                n.integer && (1.0..=31.0).contains(&n.value) && n.end == end + 1
                    || n.digits && n.integer && (1.0..=31.0).contains(&n.value)
            }) {
                day = Some(n.value as u8);
                end = n.end;
            }
            let mut yr = None;
            if day.is_some() {
                if let Some((y, e)) = year(t, end) {
                    yr = Some(y);
                    end = e;
                }
            } else {
                let prev = i.checked_sub(1).map(|p| t[p].w.as_str());
                if !matches!(
                    prev,
                    Some(
                        "in" | "of"
                            | "until"
                            | "since"
                            | "during"
                            | "by"
                            | "early"
                            | "late"
                            | "mid"
                            | "next"
                            | "last"
                            | "this"
                    )
                ) {
                    return None;
                }
                if let Some((y, e)) = year(t, end) {
                    yr = Some(y);
                    end = e;
                }
            }
            if let Some(d) = day {
                if !valid(yr, m, d) {
                    return None;
                }
            }
            return Some(Found::new(
                i,
                end,
                "date",
                date(yr, m, day),
                level,
                "date.month_first",
            ));
        }
        // ordinal "of" month [year]
        let (d, e) = ordinal(t, i)?;
        if t.get(e).is_none_or(|x| x.w != "of") {
            return None;
        }
        let m = month(&t.get(e + 1)?.w)?;
        let mut end = e + 2;
        let mut yr = None;
        if let Some((y, e2)) = year(t, end) {
            yr = Some(y);
            end = e2;
        }
        if d > 31 || !valid(yr, m, d as u8) {
            return None;
        }
        Some(Found::new(
            i,
            end,
            "date",
            date(yr, m, Some(d as u8)),
            Certainty::High,
            "date.day_first",
        ))
    }

    fn day(&self, t: &[Tok], i: usize) -> Option<Found> {
        let w = t[i].w.as_str();
        let next = |k: usize| t.get(i + k).map(|x| x.w.as_str());
        let (offset, end, part) = match w {
            "today" => (0, i + 1, None),
            "tonight" => (0, i + 1, Some("night")),
            "tomorrow" => (1, i + 1, None),
            "yesterday" => (-1, i + 1, None),
            "day" if next(1) == Some("after") && next(2) == Some("tomorrow") => (2, i + 3, None),
            "day" if next(1) == Some("before") && next(2) == Some("yesterday") => (-2, i + 3, None),
            _ => {
                let mut a = i;
                let modifier = match w {
                    "next" | "last" | "this" | "coming" => Some(w),
                    _ => None,
                };
                if modifier.is_some() {
                    a = i + 1;
                }
                let (d, plural) = weekday(&t.get(a)?.w)?;
                let mut f = Found::new(
                    i,
                    a + 1,
                    "weekday",
                    NormalizedValue::Weekday(d),
                    Certainty::High,
                    "day.weekday",
                );
                if let Some(m) = modifier {
                    f.attributes.push(attr("modifier", m));
                }
                if plural {
                    f.attributes.push(attr("recurring", "true"));
                }
                return Some(f);
            }
        };
        let mut f = Found::new(
            i,
            end,
            "relative_day",
            NormalizedValue::RelativeDay(offset),
            Certainty::High,
            "day.relative",
        );
        if let Some(p) = part {
            f.attributes.push(attr("part_of_day", p));
        }
        if let Some(r) = self.reference_date {
            let d = r.add_days(offset);
            f.attributes.push(attr(
                "resolved_date",
                format!("{:04}-{:02}-{:02}", d.year, d.month, d.day),
            ));
        }
        Some(f)
    }

    fn duration(&self, t: &[Tok], i: usize) -> Option<Found> {
        let w = t[i].w.as_str();
        let and_a_half = |k: usize| {
            t.get(k).is_some_and(|x| x.w == "and")
                && t.get(k + 1).is_some_and(|x| x.w == "a")
                && t.get(k + 2).is_some_and(|x| x.w == "half")
        };
        // "half an hour"
        if w == "half" && t.get(i + 1).is_some_and(|x| x.w == "a" || x.w == "an") {
            let u = time_unit(&t.get(i + 2)?.w)?;
            return Some(Found::new(
                i,
                i + 3,
                "duration",
                NormalizedValue::Duration(u / 2),
                Certainty::High,
                "duration.half",
            ));
        }
        let (q, mut end, level) = if w == "a" || w == "an" {
            (1.0, i + 1, Certainty::Medium)
        } else {
            let n = number(t, i)?;
            (n.value, n.end, Certainty::High)
        };
        let mut q = q;
        if and_a_half(end) {
            q += 0.5;
            end += 3;
        }
        let uw = t.get(end)?.w.as_str();
        let u = time_unit(uw)?;
        if (w == "a" || w == "an") && (uw == "second" || uw == "day") {
            // "a second" (also ordinal), "a day" (often a frequency): skip.
            return None;
        }
        end += 1;
        if and_a_half(end) {
            q += 0.5;
            end += 3;
        }
        if q <= 0.0 || q > 1.0e6 {
            return None;
        }
        let secs = (q * u as f64).round() as u64;
        Some(Found::new(
            i,
            end,
            "duration",
            NormalizedValue::Duration(secs),
            level,
            "duration.unit",
        ))
    }

    fn amount(&self, t: &[Tok], i: usize) -> Option<Found> {
        let w = t[i].w.as_str();
        if let Some(rest) = w.strip_prefix('$') {
            let v = digits(rest)?;
            return Some(Found::new(
                i,
                i + 1,
                "money",
                NormalizedValue::Money {
                    amount: v,
                    currency: "USD".into(),
                },
                Certainty::High,
                "money.symbol",
            ));
        }
        if let Some(rest) = w.strip_suffix('%') {
            let v = digits(rest)?;
            return Some(Found::new(
                i,
                i + 1,
                "percentage",
                NormalizedValue::Percentage(v),
                Certainty::High,
                "percent.symbol",
            ));
        }
        let n = number(t, i)?;
        let next = t.get(n.end)?.w.as_str();
        if let Some((code, level)) = currency(next) {
            let mut amount = n.value;
            let mut end = n.end + 1;
            if t.get(end).is_some_and(|x| x.w == "and") {
                if let Some(c) = number(t, end + 1) {
                    if c.integer
                        && c.value < 100.0
                        && t.get(c.end)
                            .is_some_and(|x| x.w == "cent" || x.w == "cents")
                    {
                        amount += c.value / 100.0;
                        end = c.end + 1;
                    }
                }
            }
            return Some(Found::new(
                i,
                end,
                "money",
                NormalizedValue::Money {
                    amount,
                    currency: code.into(),
                },
                level,
                "money.word",
            ));
        }
        if next == "percent" || (next == "per" && t.get(n.end + 1).is_some_and(|x| x.w == "cent")) {
            let end = if next == "per" { n.end + 2 } else { n.end + 1 };
            return Some(Found::new(
                i,
                end,
                "percentage",
                NormalizedValue::Percentage(n.value),
                Certainty::High,
                "percent.word",
            ));
        }
        let quantity = |unit: &str| NormalizedValue::Quantity {
            amount: n.value,
            unit: unit.into(),
        };
        if next == "pound" || next == "pounds" {
            let mut f = Found::new(
                i,
                n.end + 1,
                "quantity",
                quantity("pound"),
                Certainty::Low,
                "quantity.pound",
            );
            f.epistemic = Epistemic::Hypothesis;
            f.attributes.push(attr("ambiguity", "currency_or_weight"));
            return Some(f);
        }
        if next == "cent" || next == "cents" {
            return Some(Found::new(
                i,
                n.end + 1,
                "quantity",
                quantity("cent"),
                Certainty::Medium,
                "quantity.cent",
            ));
        }
        if let Some(u) = measure(next) {
            return Some(Found::new(
                i,
                n.end + 1,
                "quantity",
                quantity(u),
                Certainty::High,
                "quantity.measure",
            ));
        }
        if n.integer && n.value != 1.0 && counted_noun(next) {
            let mut f = Found::new(
                i,
                n.end + 1,
                "quantity",
                quantity(next),
                Certainty::Medium,
                "quantity.counted",
            );
            f.attributes.push(attr("counted_noun", "true"));
            return Some(f);
        }
        None
    }

    fn digit_run(&self, t: &[Tok], i: usize) -> Option<Found> {
        if matches!(t[i].w.as_str(), "oh" | "o") {
            return None;
        }
        let mut s = String::new();
        let mut j = i;
        while let Some(d) = t.get(j).and_then(|x| single_digit(&x.w)) {
            s.push(char::from(b'0' + d as u8));
            j += 1;
        }
        if s.len() < 4 {
            return None;
        }
        Some(Found::new(
            i,
            j,
            "digit_sequence",
            NormalizedValue::Canonical(s),
            Certainty::High,
            "digits.sequence",
        ))
    }

    fn ordinal_alone(&self, t: &[Tok], i: usize) -> Option<Found> {
        let (v, end) = ordinal(t, i)?;
        if t[i].w == "second"
            && i > 0
            && matches!(t[i - 1].w.as_str(), "a" | "one" | "per" | "each")
        {
            return None;
        }
        Some(Found::new(
            i,
            end,
            "ordinal",
            NormalizedValue::Ordinal(v),
            Certainty::High,
            "ordinal",
        ))
    }

    fn number_alone(&self, t: &[Tok], i: usize) -> Option<Found> {
        let n = number(t, i)?;
        let v = if n.integer && n.value.abs() < 9.0e15 {
            NormalizedValue::Integer(n.value as i64)
        } else {
            NormalizedValue::Decimal(n.value)
        };
        if t[i].w == "one" && n.end == i + 1 {
            let prev = i.checked_sub(1).map(|p| t[p].w.as_str());
            let next = t.get(i + 1).map(|x| x.w.as_str());
            if matches!(
                prev,
                Some(
                    "the"
                        | "that"
                        | "this"
                        | "which"
                        | "each"
                        | "every"
                        | "no"
                        | "any"
                        | "a"
                        | "another"
                        | "other"
                        | "no."
                        | "some"
                        | "big"
                        | "small"
                        | "new"
                        | "old"
                        | "red"
                        | "blue"
                        | "green"
                        | "other's"
                )
            ) || prev.is_some_and(|p| crate::text::ordinal_word(p).is_some())
                || matches!(next, Some("of" | "another" | "more"))
            {
                return None;
            }
            let mut f = Found::new(i, n.end, "number", v, Certainty::Low, "number.one");
            f.epistemic = Epistemic::Hypothesis;
            f.attributes.push(attr("ambiguity", "number_or_pronoun"));
            return Some(f);
        }
        Some(Found::new(
            i,
            n.end,
            "number",
            v,
            Certainty::High,
            if n.digits {
                "number.digits"
            } else {
                "number.words"
            },
        ))
    }

    fn profile_term(&self, t: &[Tok], folded: &[String], i: usize) -> Option<Found> {
        let p = self.profile.as_ref()?;
        for k in (1..=p.max_words.min(t.len() - i)).rev() {
            let seq = &folded[i..i + k];
            let mut matches: Vec<&ProfileEntry> =
                p.entries.iter().filter(|e| e.words == seq).collect();
            let mut plural = false;
            if matches.is_empty() {
                // Regular plural of the last word.
                if let Some(stem) = seq[k - 1].strip_suffix('s') {
                    let mut s2 = seq.to_vec();
                    s2[k - 1] = stem.to_string();
                    matches = p.entries.iter().filter(|e| e.words == s2).collect();
                    plural = !matches.is_empty();
                }
            }
            if matches.is_empty() {
                continue;
            }
            let terms: BTreeSet<usize> = matches.iter().map(|e| e.term).collect();
            let mut f = Found {
                a: i,
                b: i + k,
                category: String::new(),
                normalized: None,
                level: Certainty::Medium,
                epistemic: Epistemic::Observation,
                rule: "profile.alias",
                attributes: Vec::new(),
            };
            if terms.len() == 1 {
                let term = &p.profile.terms[matches[0].term];
                f.category = format!("profile:{}", term.category);
                f.normalized = Some(NormalizedValue::Canonical(term.canonical.clone()));
                if matches.iter().any(|e| e.canonical) && !plural {
                    f.level = Certainty::High;
                    f.rule = "profile.canonical";
                }
            } else {
                // The same surface form names several terms: an ambiguous
                // hypothesis, no normalized value (the profile cannot
                // decide which one was meant).
                let cats: BTreeSet<&str> = terms
                    .iter()
                    .map(|&ti| p.profile.terms[ti].category.as_str())
                    .collect();
                f.category = if cats.len() == 1 {
                    format!(
                        "profile:{}",
                        cats.iter().next().copied().unwrap_or_default()
                    )
                } else {
                    "profile:ambiguous".into()
                };
                f.level = Certainty::Low;
                f.epistemic = Epistemic::Hypothesis;
                f.rule = "profile.ambiguous";
                let alts: Vec<String> = terms
                    .iter()
                    .map(|&ti| {
                        format!(
                            "{}:{}",
                            p.profile.terms[ti].category, p.profile.terms[ti].canonical
                        )
                    })
                    .collect();
                f.attributes.push(attr("alternatives", alts.join("|")));
            }
            if plural {
                f.attributes.push(attr("plural", "true"));
            }
            return Some(f);
        }
        None
    }
}

fn span(t: &[Tok], a: usize, b: usize) -> TextSpan {
    TextSpan {
        start: t[a].start,
        end: t[b - 1].end,
    }
}

fn cut(chars: &[char], s: TextSpan) -> String {
    chars[s.start as usize..s.end as usize].iter().collect()
}

fn explicit(s: TextSpan, rule: &str) -> Evidence {
    Evidence {
        kind: EvidenceKind::Explicit,
        span: Some(s),
        context_item: None,
        rule: rule.into(),
    }
}

/// Word classes of referring expressions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefClass {
    Singular,
    Plural,
    Temporal,
    Locative,
    Personal,
    /// "that is right", "is that correct": refers to a proposition, never
    /// to an entity (always unresolved, reason `propositional`).
    Propositional,
}

const EVALUATIVE: [&str; 14] = [
    "right", "correct", "true", "fine", "good", "great", "wrong", "ok", "okay", "perfect", "false",
    "bad", "possible", "enough",
];

const DEMONSTRATIVE_FOLLOW: [&str; 30] = [
    "is", "was", "are", "were", "will", "would", "should", "can", "could", "please", "again",
    "too", "works", "work", "instead", "now", "to", "for", "at", "on", "in", "and", "but", "or",
    "sounds", "looks", "seems", "one", "ones", "first",
];
const COMPLEMENTIZER_PREV: [&str; 12] = [
    "think", "said", "say", "says", "know", "believe", "hope", "sure", "so", "means", "mean",
    "glad",
];

fn reference_at(t: &[Tok], i: usize) -> Option<(ReferenceKind, RefClass, usize)> {
    let w = t[i].w.as_str();
    let next = t.get(i + 1).map(|x| x.w.as_str());
    let prev = i.checked_sub(1).map(|p| t[p].w.as_str());
    match w {
        "it" | "it's" => Some((ReferenceKind::Pronoun, RefClass::Singular, i + 1)),
        "they" | "them" | "they're" => Some((ReferenceKind::Pronoun, RefClass::Plural, i + 1)),
        "that's" if next.is_some_and(|n| EVALUATIVE.contains(&n)) => {
            Some((ReferenceKind::Demonstrative, RefClass::Propositional, i + 1))
        }
        "that's" => Some((ReferenceKind::Demonstrative, RefClass::Singular, i + 1)),
        "this" | "that"
            if next.is_some_and(|n| EVALUATIVE.contains(&n))
                || (matches!(next, Some("is" | "was"))
                    && t.get(i + 2)
                        .is_some_and(|x| EVALUATIVE.contains(&x.w.as_str()))) =>
        {
            Some((ReferenceKind::Demonstrative, RefClass::Propositional, i + 1))
        }
        "this" | "that" | "these" | "those" => {
            if w == "that" && prev.is_some_and(|p| COMPLEMENTIZER_PREV.contains(&p)) {
                return None;
            }
            if next.is_some_and(|n| !DEMONSTRATIVE_FOLLOW.contains(&n)) {
                return None;
            }
            let class = if w == "these" || w == "those" {
                RefClass::Plural
            } else {
                RefClass::Singular
            };
            let end = if matches!(next, Some("one" | "ones")) {
                i + 2
            } else {
                i + 1
            };
            Some((ReferenceKind::Demonstrative, class, end))
        }
        "there" | "here" => {
            if matches!(prev, Some("hi" | "hello" | "hey" | "over" | "out")) {
                return None;
            }
            if matches!(next, Some("is" | "are" | "was" | "were" | "'s" | "will")) {
                return None;
            }
            Some((ReferenceKind::Locative, RefClass::Locative, i + 1))
        }
        "there's" | "here's" => None,
        "then" => {
            if i == 0
                || matches!(
                    prev,
                    Some("and" | "but" | "so" | "or" | "if" | "ok" | "okay" | "since" | "even")
                )
            {
                return None;
            }
            Some((ReferenceKind::Temporal, RefClass::Temporal, i + 1))
        }
        "he" | "him" | "she" | "her" => Some((ReferenceKind::Personal, RefClass::Personal, i + 1)),
        _ => None,
    }
}

fn compatible(class: RefClass, category: &str, hinted: &[String]) -> bool {
    if hinted.iter().any(|h| h == category) {
        return true;
    }
    match class {
        RefClass::Singular => matches!(category, "money" | "digit_sequence" | "percentage"),
        RefClass::Plural => category == "quantity",
        RefClass::Temporal => matches!(
            category,
            "time_of_day" | "relative_day" | "weekday" | "date"
        ),
        RefClass::Locative | RefClass::Personal | RefClass::Propositional => false,
    }
}

fn act(t: &[Tok]) -> Option<(ActDecision, Vec<String>, &'static str)> {
    if t.is_empty() {
        return None;
    }
    let mut ws: Vec<&str> = t.iter().map(|x| x.w.as_str()).collect();
    // A leading discourse marker does not decide the act: "AND SET …",
    // "SO THE TOTAL IS …" are classified on the rest.
    while ws.len() > 1 && matches!(ws[0], "and" | "so" | "also" | "but" | "well" | "now") {
        ws.remove(0);
    }
    let first = ws[0];
    let has = |p: &[&str]| ws.windows(p.len()).any(|w| w == p);
    let starts = |p: &[&str]| ws.starts_with(p);
    let mut cues: BTreeSet<&'static str> = BTreeSet::new();

    const GREET: [&str; 12] = [
        "hi",
        "hello",
        "hey",
        "bye",
        "goodbye",
        "good",
        "morning",
        "afternoon",
        "evening",
        "night",
        "there",
        "everyone",
    ];
    let greeting = matches!(first, "hi" | "hello" | "hey" | "bye" | "goodbye")
        || starts(&["good", "morning"])
        || starts(&["good", "afternoon"])
        || starts(&["good", "evening"])
        || starts(&["good", "night"]);
    let only_greeting = greeting && ws.iter().all(|w| GREET.contains(w));

    const WH: [&str; 9] = [
        "what", "when", "where", "who", "whom", "whose", "why", "how", "which",
    ];
    const AUX: [&str; 26] = [
        "is", "are", "was", "were", "do", "does", "did", "can", "could", "will", "would", "should",
        "shall", "may", "might", "have", "has", "had", "am", "isn't", "aren't", "don't", "doesn't",
        "didn't", "can't", "won't",
    ];
    if (WH.contains(&first) || (AUX.contains(&first) && ws.len() >= 2))
        || t.iter().any(|x| x.question_mark)
    {
        cues.insert("question");
    }
    const IMPERATIVE: [&str; 48] = [
        "meet", "keep", "use", "try", "look", "listen", "say", "help", "turn", "set", "send",
        "call", "book", "add", "remove", "open", "close", "move", "change", "show", "tell", "give",
        "cancel", "schedule", "play", "stop", "start", "put", "make", "take", "bring", "check",
        "find", "read", "remind", "order", "switch", "delete", "update", "write", "text", "email",
        "pay", "buy", "get", "repeat", "wait", "go",
    ];
    let polite = matches!(ws.as_slice(), [a, b, ..] if matches!(*a, "can" | "could" | "would" | "will") && *b == "you")
        && ws.len() >= 3;
    if has(&["please"])
        || IMPERATIVE.contains(&first)
        || polite
        || starts(&["i", "want"])
        || starts(&["i'd", "like"])
        || starts(&["i", "would", "like"])
        || starts(&["i", "need"])
        || starts(&["let's"])
        || starts(&["let", "me"])
        || starts(&["don't"]) && ws.len() >= 2
    {
        cues.insert("request");
    }
    // "would you like …" is an offer phrased as a question.
    let offer = ws.starts_with(&["would", "you", "like"]) || ws.starts_with(&["do", "you", "want"]);
    if polite && !offer {
        cues.remove("question");
    }
    if offer {
        cues.remove("request");
        cues.insert("question");
    }
    const YES: [&str; 10] = [
        "yes",
        "yeah",
        "yep",
        "yup",
        "correct",
        "exactly",
        "sure",
        "absolutely",
        "definitely",
        "affirmative",
    ];
    let confirm = YES.contains(&first)
        || starts(&["that's", "right"])
        || starts(&["that", "is", "right"])
        || starts(&["that's", "correct"])
        || starts(&["sounds", "good"])
        || starts(&["that", "works"]);
    if confirm {
        cues.insert("confirmation");
    }
    let refuse = matches!(first, "no" | "nope" | "nah")
        || starts(&["not", "now"])
        || starts(&["never", "mind"])
        || starts(&["i", "don't", "want"])
        || starts(&["i", "won't"])
        || starts(&["i'd", "rather", "not"]);
    if refuse {
        cues.insert("refusal");
    }
    const ACK: [&str; 9] = [
        "ok",
        "okay",
        "alright",
        "thanks",
        "cool",
        "great",
        "understood",
        "noted",
        "gotcha",
    ];
    if ACK.contains(&first)
        || starts(&["got", "it"])
        || starts(&["thank", "you"])
        || starts(&["i", "see"])
        || starts(&["uh", "huh"])
        || starts(&["mm", "hmm"])
        || starts(&["all", "right"])
    {
        cues.insert("acknowledgement");
    }
    let strong_correction = has(&["i", "mean"])
        || has(&["i", "meant"])
        || has(&["i", "said"])
        || has(&["correction"])
        || (has(&["not"]) && has(&["but"]));
    let weak_correction = has(&["actually"]) || has(&["instead"]) || has(&["rather"]) && !refuse;

    // Precedence rules (documented in semantic.md):
    // 1. a strong correction cue decides;
    if strong_correction {
        let alts: Vec<String> = cues.iter().map(|s| (*s).to_string()).collect();
        return Some((
            ActDecision::Labeled {
                label: "correction".into(),
            },
            alts,
            "act.correction",
        ));
    }
    // 2. "yes please" / "no thanks" are confirmation / refusal;
    if confirm && ws.len() == 2 && ws[1] == "please" {
        cues.remove("request");
    }
    if refuse && (starts(&["no", "thanks"]) || starts(&["no", "thank", "you"])) {
        cues.remove("acknowledgement");
    }
    // 3. a weak correction cue counts only alone.
    if cues.is_empty() && weak_correction {
        return Some((
            ActDecision::Labeled {
                label: "correction".into(),
            },
            Vec::new(),
            "act.correction_weak",
        ));
    }
    if cues.len() > 1 {
        let alts = cues.iter().map(|s| (*s).to_string()).collect();
        return Some((
            ActDecision::Abstained {
                reason: ActAbstention::Ambiguous,
            },
            alts,
            "act.ambiguous",
        ));
    }
    if let Some(c) = cues.iter().next() {
        return Some((
            ActDecision::Labeled {
                label: (*c).to_string(),
            },
            Vec::new(),
            "act.cue",
        ));
    }
    if only_greeting || greeting {
        return Some((
            ActDecision::Abstained {
                reason: ActAbstention::OutOfTaxonomy,
            },
            Vec::new(),
            "act.out_of_taxonomy",
        ));
    }
    const SUBJECT: [&str; 22] = [
        "i", "we", "you", "he", "she", "it", "they", "there", "this", "that", "my", "our", "the",
        "it's", "i'm", "we're", "they're", "there's", "that's", "i've", "i'll", "we'll",
    ];
    if SUBJECT.contains(&first) && ws.len() >= 3 {
        return Some((
            ActDecision::Labeled {
                label: "statement".into(),
            },
            Vec::new(),
            "act.statement",
        ));
    }
    Some((
        ActDecision::Abstained {
            reason: ActAbstention::NoCue,
        },
        Vec::new(),
        "act.no_cue",
    ))
}

impl SemanticAdapter for RulesAdapter {
    fn analyze(
        &mut self,
        input: &SemanticInput,
        context: &ContextView,
    ) -> Result<Vec<Draft>, CeError> {
        if input.language.as_ref().is_some_and(|l| l.primary() != "en") {
            return Ok(Vec::new());
        }
        let chars: Vec<char> = input.text.chars().collect();
        let t = tokenize(&input.text);
        let folded: Vec<String> = t.iter().map(|x| fold(&x.w)).collect();
        let mut used = vec![false; t.len()];
        let mut found: Vec<Found> = Vec::new();
        if self.entities {
            type Pass<'a> = Box<dyn Fn(&RulesAdapter, &[Tok], usize) -> Option<Found> + 'a>;
            let passes: Vec<Pass<'_>> = vec![
                Box::new(|s, t, i| s.profile_term(t, &folded, i)),
                Box::new(|s, t, i| s.digit_run(t, i)),
                Box::new(|s, t, i| s.time(t, i)),
                Box::new(|s, t, i| s.date(t, i)),
                Box::new(|s, t, i| s.day(t, i)),
                Box::new(|s, t, i| s.duration(t, i)),
                Box::new(|s, t, i| s.amount(t, i)),
                Box::new(|s, t, i| s.ordinal_alone(t, i)),
                Box::new(|s, t, i| s.number_alone(t, i)),
            ];
            for pass in &passes {
                let mut i = 0;
                while i < t.len() {
                    if used[i] {
                        i += 1;
                        continue;
                    }
                    match pass(self, &t, i) {
                        Some(f)
                            if f.b > f.a
                                && f.b <= t.len()
                                && !used[f.a..f.b].iter().any(|u| *u) =>
                        {
                            for u in &mut used[f.a..f.b] {
                                *u = true;
                            }
                            i = f.b;
                            found.push(f);
                        }
                        _ => i += 1,
                    }
                }
            }
        }
        found.sort_by_key(|f| f.a);
        let mut drafts: Vec<Draft> = Vec::new();
        let mut entity_tok: Vec<(usize, usize, usize)> = Vec::new(); // (a, b, draft index)
        for f in &found {
            let s = span(&t, f.a, f.b);
            let mention = cut(&chars, s);
            let mut evidence = vec![explicit(s, f.rule)];
            if f.category.starts_with("profile:") {
                evidence[0].kind = EvidenceKind::Profile;
            }
            entity_tok.push((f.a, f.b, drafts.len()));
            drafts.push(Draft {
                key: format!("entity:{}:{}", f.category, fold(&mention)),
                payload: DraftPayload::Entity(Entity {
                    category: f.category.clone(),
                    mention,
                    normalized: f.normalized.clone(),
                    attributes: f.attributes.clone(),
                }),
                span: s,
                epistemic: f.epistemic,
                uncertainty: Uncertainty::tier(f.level),
                evidence,
            });
        }
        if self.relations {
            let mut rel = Vec::new();
            for (x, &(_, xb, xd)) in entity_tok.iter().enumerate() {
                let xc = category_of(&drafts[xd]);
                for &(ya, _, yd) in entity_tok.iter().skip(x + 1) {
                    let yc = category_of(&drafts[yd]);
                    let gap = ya.saturating_sub(xb);
                    let day = |c: &str| matches!(c, "relative_day" | "weekday" | "date");
                    let kind = if xc == "number" && yc.starts_with("profile:") && gap == 0 {
                        Some(("quantity_of", xd, yd))
                    } else if gap <= 2 && xc == "time_of_day" && day(&yc) {
                        Some(("on_day", xd, yd))
                    } else if gap <= 2 && day(&xc) && yc == "time_of_day" {
                        Some(("on_day", yd, xd))
                    } else {
                        None
                    };
                    if let Some((kind, from, to)) = kind {
                        rel.push((kind, from, to));
                    }
                }
            }
            for (kind, from, to) in rel {
                let s = TextSpan {
                    start: drafts[from].span.start.min(drafts[to].span.start),
                    end: drafts[from].span.end.max(drafts[to].span.end),
                };
                let lvl = drafts[from]
                    .certainty()
                    .min(drafts[to].certainty())
                    .min(Certainty::Medium);
                drafts.push(Draft {
                    key: format!("relation:{kind}:{}:{}", drafts[from].key, drafts[to].key),
                    payload: DraftPayload::Relation {
                        kind: kind.into(),
                        from,
                        to: RelationTarget::Draft(to),
                    },
                    span: s,
                    epistemic: Epistemic::Hypothesis,
                    uncertainty: Uncertainty::tier(lvl),
                    evidence: vec![Evidence {
                        kind: EvidenceKind::Inferred,
                        span: Some(s),
                        context_item: None,
                        rule: format!("relation.{kind}"),
                    }],
                });
            }
        }
        if self.references {
            let mut i = 0;
            while i < t.len() {
                if used[i] {
                    i += 1;
                    continue;
                }
                let Some((kind, class, end)) = reference_at(&t, i) else {
                    i += 1;
                    continue;
                };
                let s = span(&t, i, end);
                let mention = cut(&chars, s);
                let referent = t[i].w.trim_end_matches("'s").trim_end_matches("'re");
                let hinted = self
                    .profile
                    .as_ref()
                    .map(|p| p.hinted(referent))
                    .unwrap_or_default();
                let candidates: Vec<&ContextItem> = context
                    .items
                    .iter()
                    .filter(|c| compatible(class, &c.category, &hinted))
                    .filter(|c| {
                        c.source.utterance != input.utterance || c.source.span.end <= s.start
                    })
                    .collect();
                let resolution = if class == RefClass::Propositional {
                    Resolution::Unresolved {
                        reason: "propositional".into(),
                    }
                } else if context.disabled {
                    Resolution::Unresolved {
                        reason: "context_disabled".into(),
                    }
                } else {
                    match candidates.as_slice() {
                        [] => Resolution::Unresolved {
                            reason: "no_compatible_candidate".into(),
                        },
                        [one] => Resolution::Resolved {
                            target: one.id.clone(),
                        },
                        _ => Resolution::Ambiguous,
                    }
                };
                let resolved = matches!(resolution, Resolution::Resolved { .. });
                let mut evidence = vec![explicit(s, &format!("reference.{}", kind_name(kind)))];
                for c in &candidates {
                    evidence.push(Evidence {
                        kind: EvidenceKind::Context,
                        span: None,
                        context_item: Some(c.id.clone()),
                        rule: "reference.candidate".into(),
                    });
                }
                let target = match &resolution {
                    Resolution::Resolved { target } => Some(target.clone()),
                    _ => None,
                };
                let idx = drafts.len();
                drafts.push(Draft {
                    key: format!("reference:{}", fold(&mention)),
                    payload: DraftPayload::Reference(ReferenceResolution {
                        mention,
                        kind,
                        candidates: candidates
                            .iter()
                            .map(|c| ReferenceCandidate {
                                item: c.id.clone(),
                                category: c.category.clone(),
                                mention: c.mention.clone(),
                                rule: "reference.compatible_category".into(),
                            })
                            .collect(),
                        resolution,
                    }),
                    span: s,
                    epistemic: if resolved {
                        Epistemic::ResolvedInterpretation
                    } else {
                        Epistemic::Hypothesis
                    },
                    uncertainty: Uncertainty::tier(if resolved {
                        Certainty::Medium
                    } else {
                        Certainty::Low
                    }),
                    evidence,
                });
                if let (Some(target), true) = (target, self.relations) {
                    drafts.push(Draft {
                        key: format!("relation:refers_to:{}:{}", drafts[idx].key, target.0),
                        payload: DraftPayload::Relation {
                            kind: "refers_to".into(),
                            from: idx,
                            to: RelationTarget::Context(target.clone()),
                        },
                        span: s,
                        epistemic: Epistemic::ResolvedInterpretation,
                        uncertainty: Uncertainty::tier(Certainty::Medium),
                        evidence: vec![Evidence {
                            kind: EvidenceKind::Context,
                            span: Some(s),
                            context_item: Some(target),
                            rule: "relation.refers_to".into(),
                        }],
                    });
                }
                i = end;
            }
        }
        if self.acts {
            if let Some((decision, alternatives, rule)) = act(&t) {
                // A configured subset: labels outside it abstain.
                let (decision, rule) = match (&decision, &self.act_labels) {
                    (ActDecision::Labeled { label }, Some(allowed)) if !allowed.contains(label) => {
                        (
                            ActDecision::Abstained {
                                reason: ActAbstention::OutOfTaxonomy,
                            },
                            "act.outside_configured_subset",
                        )
                    }
                    _ => (decision, rule),
                };
                let alternatives = match &self.act_labels {
                    Some(allowed) => alternatives
                        .into_iter()
                        .filter(|a| allowed.contains(a))
                        .collect(),
                    None => alternatives,
                };
                let s = span(&t, 0, t.len());
                let labeled = matches!(decision, ActDecision::Labeled { .. });
                drafts.push(Draft {
                    key: "act".into(),
                    payload: DraftPayload::Act(CommunicativeAct {
                        taxonomy: ACT_TAXONOMY.into(),
                        taxonomy_version: ACT_TAXONOMY_VERSION.into(),
                        decision,
                        alternatives,
                    }),
                    span: s,
                    epistemic: Epistemic::Hypothesis,
                    uncertainty: Uncertainty::tier(if labeled {
                        Certainty::Medium
                    } else {
                        Certainty::Low
                    }),
                    evidence: vec![Evidence {
                        kind: EvidenceKind::Inferred,
                        span: Some(s),
                        context_item: None,
                        rule: rule.into(),
                    }],
                });
            }
        }
        Ok(drafts)
    }
}

fn category_of(d: &Draft) -> String {
    match &d.payload {
        DraftPayload::Entity(e) => e.category.clone(),
        _ => String::new(),
    }
}

fn kind_name(k: ReferenceKind) -> &'static str {
    match k {
        ReferenceKind::Pronoun => "pronoun",
        ReferenceKind::Demonstrative => "demonstrative",
        ReferenceKind::Locative => "locative",
        ReferenceKind::Temporal => "temporal",
        ReferenceKind::Personal => "personal",
    }
}
