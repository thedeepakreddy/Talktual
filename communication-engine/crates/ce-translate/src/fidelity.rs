//! Terminology matching, protected spans and fidelity checks.
//!
//! * **Term matching** is whole-word (a term never matches inside another
//!   word: "room" does not match "roommate"), case-insensitive unless the
//!   term says otherwise, longest match first, non-overlapping. This is the
//!   guard against harmful over-application.
//! * **Protected spans** (digit sequences and `do_not_translate` terms) are
//!   replaced by `⟦n⟧` placeholders only for adapters that declare
//!   `placeholders`; they are restored after translation. A lost
//!   placeholder is reported (`protected_span_lost`), never guessed back in.
//! * **Fidelity checks** never edit the output; they raise uncertainty
//!   flags: a source digit sequence missing from the output
//!   (`number_mismatch`), a source term whose target term is missing
//!   (`term_missing`).
//!
//! Inflection is not handled: a term matches its listed form only (an
//! inflected target form will be flagged `term_missing`; documented
//! limitation).

use ce_core::translation::{Term, TermKind};

/// A match of a term in text, in byte offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TermMatch {
    /// Byte start.
    pub start: usize,
    /// Byte end.
    pub end: usize,
    /// Index into the term list.
    pub term: usize,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '-'
}

fn boundary_before(text: &str, at: usize) -> bool {
    text[..at]
        .chars()
        .next_back()
        .is_none_or(|c| !is_word_char(c))
}

fn boundary_after(text: &str, at: usize) -> bool {
    text[at..].chars().next().is_none_or(|c| !is_word_char(c))
}

/// Find whole-word, non-overlapping term matches (longest first).
pub fn find_terms(text: &str, terms: &[Term]) -> Vec<TermMatch> {
    let lower = text.to_lowercase();
    // Lowercasing can change byte lengths for some scripts; fall back to
    // case-sensitive matching there rather than mis-mapping offsets.
    let same_len = lower.len() == text.len();
    let mut order: Vec<usize> = (0..terms.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(terms[i].source.trim().chars().count()));
    let mut taken: Vec<(usize, usize)> = Vec::new();
    let mut out = Vec::new();
    for i in order {
        let t = &terms[i];
        let needle_raw = t.source.trim();
        if needle_raw.is_empty() {
            continue;
        }
        let (hay, needle) = if t.case_sensitive || !same_len {
            (text, needle_raw.to_string())
        } else {
            (lower.as_str(), needle_raw.to_lowercase())
        };
        let mut from = 0;
        while let Some(pos) = hay[from..].find(&needle) {
            let s = from + pos;
            let e = s + needle.len();
            from = s + needle.len().max(1);
            if !boundary_before(text, s) || !boundary_after(text, e) {
                continue;
            }
            if taken.iter().any(|(a, b)| s < *b && *a < e) {
                continue;
            }
            taken.push((s, e));
            out.push(TermMatch {
                start: s,
                end: e,
                term: i,
            });
        }
    }
    out.sort_by_key(|m| m.start);
    out
}

/// Digit sequences (with internal `.`/`,`/`:` separators) in text.
pub fn digit_runs(text: &str) -> Vec<(usize, usize)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let s = i;
            while i < b.len()
                && (b[i].is_ascii_digit()
                    || (matches!(b[i], b'.' | b',' | b':')
                        && i + 1 < b.len()
                        && b[i + 1].is_ascii_digit()))
            {
                i += 1;
            }
            out.push((s, i));
        } else {
            i += 1;
        }
    }
    out
}

/// The placeholder for span `n`.
pub fn placeholder(n: usize) -> String {
    format!("⟦{n}⟧")
}

/// Text with protected spans replaced by placeholders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Protected {
    /// Masked text.
    pub text: String,
    /// Original span texts, by placeholder number.
    pub spans: Vec<String>,
    /// How many were `do_not_translate` terms.
    pub terms: u32,
}

/// Replace digit runs and `do_not_translate` term matches by placeholders.
pub fn protect(text: &str, terms: &[Term]) -> Protected {
    let mut ranges: Vec<(usize, usize, bool)> = find_terms(text, terms)
        .into_iter()
        .filter(|m| terms[m.term].kind == TermKind::DoNotTranslate)
        .map(|m| (m.start, m.end, true))
        .collect();
    for (s, e) in digit_runs(text) {
        if !ranges.iter().any(|(a, b, _)| s < *b && *a < e) {
            ranges.push((s, e, false));
        }
    }
    ranges.sort_by_key(|r| r.0);
    let mut out = String::with_capacity(text.len());
    let mut spans = Vec::new();
    let mut last = 0;
    let mut n_terms = 0;
    for (s, e, is_term) in ranges {
        out.push_str(&text[last..s]);
        out.push_str(&placeholder(spans.len()));
        spans.push(text[s..e].to_string());
        n_terms += u32::from(is_term);
        last = e;
    }
    out.push_str(&text[last..]);
    Protected {
        text: out,
        spans,
        terms: n_terms,
    }
}

/// Result of restoring placeholders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Restored {
    /// Text with placeholders restored.
    pub text: String,
    /// Some placeholder was missing or duplicated.
    pub lost: bool,
    /// The output contains a placeholder that was never issued (malformed).
    pub foreign: bool,
}

/// Restore placeholders. Each must appear exactly once.
pub fn restore(output: &str, spans: &[String]) -> Restored {
    let mut text = output.to_string();
    let mut lost = false;
    for (n, s) in spans.iter().enumerate() {
        let p = placeholder(n);
        let count = text.matches(&p).count();
        if count != 1 {
            lost = true;
        }
        text = text.replace(&p, s);
    }
    let foreign = text.contains('⟦') || text.contains('⟧');
    Restored {
        text,
        lost,
        foreign,
    }
}

/// Source digit runs missing from the output (exact string match).
pub fn numbers_missing(source: &str, output: &str) -> bool {
    digit_runs(source)
        .into_iter()
        .any(|(s, e)| !output.contains(&source[s..e]))
}

/// Source `translate` terms whose target term is absent from the output
/// (case-insensitive, whole word).
pub fn terms_missing(source: &str, output: &str, terms: &[Term]) -> (u32, bool) {
    let found = find_terms(source, terms);
    let mut applied = 0;
    let mut missing = false;
    for m in &found {
        let t = &terms[m.term];
        if t.kind != TermKind::Translate {
            continue;
        }
        applied += 1;
        let target = Term {
            source: t.target.clone(),
            target: String::new(),
            kind: TermKind::Translate,
            case_sensitive: false,
        };
        if find_terms(output, std::slice::from_ref(&target)).is_empty() {
            missing = true;
        }
    }
    (applied, missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(s: &str, t: &str, kind: TermKind) -> Term {
        Term {
            source: s.into(),
            target: t.into(),
            kind,
            case_sensitive: false,
        }
    }

    #[test]
    fn whole_word_longest_first_matching_never_overapplies() {
        let terms = vec![
            term("room", "habitación", TermKind::Translate),
            term(
                "room service",
                "servicio de habitaciones",
                TermKind::Translate,
            ),
        ];
        let m = find_terms("Room service for my roommate's room, please", &terms);
        let words: Vec<_> = m.iter().map(|x| (x.term, x.start)).collect();
        assert_eq!(
            words,
            vec![(1, 0), (0, 31)],
            "'roommate' is not 'room'; the longer term wins"
        );
        let cs = vec![Term {
            case_sensitive: true,
            ..term("Bill", "Bill", TermKind::DoNotTranslate)
        }];
        assert!(find_terms("please bring the bill", &cs).is_empty());
        assert_eq!(find_terms("Bill is here", &cs).len(), 1);
    }

    #[test]
    fn protection_round_trips_and_detects_loss() {
        let terms = vec![term("Talktual", "", TermKind::DoNotTranslate)];
        let p = protect("Talktual room 12, at 7:30 or 3.5", &terms);
        assert_eq!(p.text, "⟦0⟧ room ⟦1⟧, at ⟦2⟧ or ⟦3⟧");
        assert_eq!((p.spans.len(), p.terms), (4, 1));
        let ok = restore("⟦0⟧ habitación ⟦1⟧, a las ⟦2⟧ o ⟦3⟧", &p.spans);
        assert_eq!(ok.text, "Talktual habitación 12, a las 7:30 o 3.5");
        assert!(!ok.lost && !ok.foreign);
        let lost = restore("habitación ⟦1⟧ ⟦1⟧ ⟦2⟧ ⟦3⟧", &p.spans);
        assert!(lost.lost);
        assert!(restore("x ⟦9⟧", &[]).foreign);
    }

    #[test]
    fn fidelity_checks_flag_but_never_edit() {
        assert!(numbers_missing(
            "room 12 at 7:30",
            "habitación 21 a las 7:30"
        ));
        assert!(!numbers_missing("room 12", "habitación 12"));
        let terms = vec![term("check-in", "registro", TermKind::Translate)];
        assert_eq!(
            terms_missing("Where is check-in?", "¿Dónde está el registro?", &terms),
            (1, false)
        );
        assert_eq!(
            terms_missing("Where is check-in?", "¿Dónde está la recepción?", &terms),
            (1, true)
        );
        assert_eq!(terms_missing("no term here", "nada", &terms), (0, false));
    }
}
