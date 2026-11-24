//! Streaming transcript semantics: stable-prefix commits, revisions
//! (snapshot + delta), and the consumer-side assembler.
//!
//! **Producer side** ([`UtteranceText`]): each adapter hypothesis is a full
//! hypothesis of the utterance so far. A word prefix becomes *stable* when it
//! is unchanged across `stable_after_hypotheses` consecutive hypotheses or
//! for `stable_after_ms` of audio; the stable prefix minus `holdback_words`
//! is *committed*. Committed text never shrinks before the final. Later
//! hypotheses that contradict committed words do not rewrite it (the
//! contradiction is counted); only the **final** may amend committed text,
//! and then says so (`commit_amended`). Corrections in the provisional tail
//! are never suppressed.
//!
//! **Consumer side** ([`TranscriptAssembler`]): applies revisions in order,
//! rejects stale and duplicate revisions, falls back to the snapshot after a
//! gap, and refuses anything after a final or cancellation.

use ce_core::speech::{
    RevisionStatus, StabilityPolicy, TranscriptDelta, TranscriptRevision, TranscriptText,
};

fn words(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_owned).collect()
}

fn common_prefix(a: &[String], b: &[String]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// Delta from `prev` to `next` (character based, Unicode scalar values).
pub fn delta(prev: &TranscriptText, next: &TranscriptText) -> TranscriptDelta {
    let keep = prev
        .committed
        .chars()
        .zip(next.committed.chars())
        .take_while(|(a, b)| a == b)
        .count();
    TranscriptDelta {
        keep_committed_chars: keep as u32,
        append_committed: next.committed.chars().skip(keep).collect(),
        provisional: next.provisional.clone(),
    }
}

/// Apply a delta to a snapshot.
pub fn apply(prev: &TranscriptText, d: &TranscriptDelta) -> TranscriptText {
    let mut committed: String = prev
        .committed
        .chars()
        .take(d.keep_committed_chars as usize)
        .collect();
    committed.push_str(&d.append_committed);
    TranscriptText {
        committed,
        provisional: d.provisional.clone(),
    }
}

/// Producer-side state of one utterance's text.
#[derive(Clone, Debug)]
pub struct UtteranceText {
    policy: StabilityPolicy,
    /// Last hypothesis words.
    last: Vec<String>,
    /// For each position of `last`: consecutive hypotheses it has survived
    /// (as part of an unchanged prefix) and the audio time it last changed.
    survived: Vec<u32>,
    changed_at_ms: Vec<u64>,
    committed: Vec<String>,
    snapshot: TranscriptText,
    revision: u32,
    contradictions: u32,
    finished: bool,
}

/// A revision decided by [`UtteranceText`] (metadata is added by the caller).
#[derive(Clone, Debug, PartialEq)]
pub struct TextRevision {
    /// Revision number.
    pub revision: u32,
    /// Snapshot.
    pub text: TranscriptText,
    /// Delta from the previous revision.
    pub delta: TranscriptDelta,
    /// Committed words (count) after this revision.
    pub committed_words: usize,
    /// Final changed committed text.
    pub commit_amended: bool,
}

impl UtteranceText {
    /// A new utterance.
    pub fn new(policy: StabilityPolicy) -> Self {
        Self {
            policy,
            last: Vec::new(),
            survived: Vec::new(),
            changed_at_ms: Vec::new(),
            committed: Vec::new(),
            snapshot: TranscriptText::default(),
            revision: 0,
            contradictions: 0,
            finished: false,
        }
    }

    /// Hypotheses that contradicted committed words.
    pub fn contradictions(&self) -> u32 {
        self.contradictions
    }

    /// Current snapshot.
    pub fn snapshot(&self) -> &TranscriptText {
        &self.snapshot
    }

    /// Revisions emitted.
    pub fn revisions(&self) -> u32 {
        self.revision
    }

    fn emit(&mut self, next: TranscriptText, amended: bool) -> TextRevision {
        let d = delta(&self.snapshot, &next);
        let rev = TextRevision {
            revision: self.revision,
            text: next.clone(),
            delta: d,
            committed_words: self.committed.len(),
            commit_amended: amended,
        };
        self.snapshot = next;
        self.revision += 1;
        rev
    }

    /// Feed a partial hypothesis at audio time `now_ms` (utterance-relative).
    /// Returns a revision when the snapshot changed (identical hypotheses are
    /// deduplicated).
    pub fn partial(&mut self, text: &str, now_ms: u64) -> Option<TextRevision> {
        if self.finished {
            return None;
        }
        let w = words(text);
        let keep = common_prefix(&self.last, &w);
        let mut survived = Vec::with_capacity(w.len());
        let mut changed = Vec::with_capacity(w.len());
        for i in 0..w.len() {
            if i < keep {
                survived.push(self.survived[i].saturating_add(1));
                changed.push(self.changed_at_ms[i]);
            } else {
                survived.push(1);
                changed.push(now_ms);
            }
        }
        // Stable prefix length.
        let p = &self.policy;
        let stable = (0..w.len())
            .take_while(|&i| {
                (p.stable_after_hypotheses > 0 && survived[i] >= p.stable_after_hypotheses)
                    || (p.stable_after_ms > 0
                        && now_ms.saturating_sub(changed[i]) >= u64::from(p.stable_after_ms))
            })
            .count();
        let limit = w.len().saturating_sub(p.holdback_words as usize);
        let target = stable.min(limit);
        let agrees = common_prefix(&self.committed, &w) == self.committed.len();
        if !agrees {
            self.contradictions += 1;
        } else if target > self.committed.len() {
            self.committed = w[..target].to_vec();
        }
        self.last = w;
        self.survived = survived;
        self.changed_at_ms = changed;
        // The provisional tail is the hypothesis after the committed words
        // (when the hypothesis contradicts committed words, after the same
        // number of words: the final will settle it).
        let n = self.committed.len().min(self.last.len());
        let next = TranscriptText {
            committed: self.committed.join(" "),
            provisional: self.last[n..].join(" "),
        };
        (next != self.snapshot).then(|| self.emit(next, false))
    }

    /// The final hypothesis: everything is committed. Always returns a
    /// revision (even if the text did not change, the status does).
    pub fn finalize(&mut self, text: &str) -> TextRevision {
        let w = words(text);
        let amended = common_prefix(&self.committed, &w) < self.committed.len();
        self.committed = w;
        self.finished = true;
        let next = TranscriptText {
            committed: self.committed.join(" "),
            provisional: String::new(),
        };
        self.emit(next, amended)
    }

    /// Cancel: keep what was committed, drop the provisional tail.
    pub fn cancel(&mut self) -> TextRevision {
        self.finished = true;
        let next = TranscriptText {
            committed: self.committed.join(" "),
            provisional: String::new(),
        };
        self.emit(next, false)
    }
}

/// Why a revision was not applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// Its number is not greater than the last applied (stale/duplicate).
    Stale,
    /// The utterance already ended (final or cancelled).
    Ended,
    /// Another utterance than the one being assembled.
    WrongUtterance,
}

/// Consumer-side reconstruction of one utterance.
#[derive(Clone, Debug, Default)]
pub struct TranscriptAssembler {
    utterance: Option<u64>,
    last: Option<u32>,
    text: TranscriptText,
    ended: Option<RevisionStatus>,
    gaps: u32,
}

impl TranscriptAssembler {
    /// Empty.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current text.
    pub fn text(&self) -> &TranscriptText {
        &self.text
    }

    /// Status if ended.
    pub fn ended(&self) -> Option<RevisionStatus> {
        self.ended
    }

    /// Revisions skipped (snapshots used instead of deltas).
    pub fn gaps(&self) -> u32 {
        self.gaps
    }

    /// Apply one revision. Deltas are used when the revision directly
    /// follows the last applied one; otherwise the snapshot.
    pub fn apply(&mut self, r: &TranscriptRevision) -> Result<(), Rejected> {
        match self.utterance {
            Some(u) if u != r.meta.utterance => return Err(Rejected::WrongUtterance),
            _ => {}
        }
        if self.ended.is_some() {
            return Err(Rejected::Ended);
        }
        if self.last.is_some_and(|l| r.revision <= l) {
            return Err(Rejected::Stale);
        }
        let contiguous = match self.last {
            None => r.revision == 0,
            Some(l) => r.revision == l + 1,
        };
        self.text = if contiguous {
            apply(&self.text, &r.delta)
        } else {
            self.gaps += 1;
            r.text.clone()
        };
        self.utterance = Some(r.meta.utterance);
        self.last = Some(r.revision);
        if r.status != RevisionStatus::Partial {
            self.ended = Some(r.status);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(n: u32, ms: u32, hold: u32) -> StabilityPolicy {
        StabilityPolicy {
            stable_after_hypotheses: n,
            stable_after_ms: ms,
            holdback_words: hold,
        }
    }

    #[test]
    fn commits_grow_only_from_stable_prefixes_and_keep_the_holdback() {
        let mut u = UtteranceText::new(policy(2, 0, 1));
        let r0 = u.partial("the", 100).unwrap();
        assert_eq!(
            (r0.text.committed.as_str(), r0.text.provisional.as_str()),
            ("", "the")
        );
        // 'the' has survived 2 hypotheses and is not the held-back last word.
        let r1 = u.partial("the cat", 200).unwrap();
        assert_eq!(
            (r1.text.committed.as_str(), r1.text.provisional.as_str()),
            ("the", "cat")
        );
        let r1b = u.partial("the cat sat", 300).unwrap();
        assert_eq!(r1b.text.committed, "the cat");
        assert_eq!(r1b.text.provisional, "sat");
        // Identical hypothesis: no revision (dedupe) — but stability grows.
        assert!(u.partial("the cat sat", 400).is_none());
        let r2 = u.partial("the cat sat on", 500).unwrap();
        assert_eq!(r2.text.committed, "the cat sat");
        assert_eq!(r2.delta.keep_committed_chars, 7);
        assert_eq!(r2.delta.append_committed, " sat");
        let f = u.finalize("the cat sat on the mat");
        assert_eq!(
            (f.text.committed.as_str(), f.commit_amended),
            ("the cat sat on the mat", false)
        );
        assert!(u.partial("more", 900).is_none(), "nothing after the final");
    }

    #[test]
    fn time_rule_contradictions_and_amending_finals() {
        let mut u = UtteranceText::new(policy(0, 300, 0));
        u.partial("I scream", 0);
        assert_eq!(u.partial("I scream", 100), None);
        let r = u.partial("I scream for", 400).unwrap();
        assert_eq!(r.text.committed, "I scream");
        // A later hypothesis contradicts committed words: committed text is
        // kept, the contradiction counted, the provisional tail follows.
        let r = u.partial("ice cream for you", 500).unwrap();
        assert_eq!(
            (r.text.committed.as_str(), r.text.provisional.as_str()),
            ("I scream", "for you")
        );
        assert_eq!(u.contradictions(), 1);
        // Only the final may amend committed text, and says so.
        let f = u.finalize("ice cream for you");
        assert!(f.commit_amended);
        assert_eq!(f.delta.keep_committed_chars, 0);
        assert_eq!(f.delta.append_committed, "ice cream for you");
    }

    #[test]
    fn deltas_reproduce_snapshots_and_cancel_keeps_commits() {
        let mut u = UtteranceText::new(policy(2, 0, 1));
        let mut text = TranscriptText::default();
        let mut revs = Vec::new();
        for h in [
            "hello",
            "hello wor",
            "hello world",
            "hello world how",
            "hello world how are",
            "hello world how are you",
        ] {
            if let Some(r) = u.partial(h, 0) {
                revs.push(r);
            }
        }
        revs.push(u.cancel());
        for (i, r) in revs.iter().enumerate() {
            assert_eq!(r.revision as usize, i, "gapless numbering");
            text = apply(&text, &r.delta);
            assert_eq!(text, r.text);
        }
        let last = revs.last().unwrap();
        assert!(last.text.provisional.is_empty());
        assert!(last.text.committed.starts_with("hello world"));
    }

    fn rev(
        utt: u64,
        n: u32,
        status: RevisionStatus,
        text: TranscriptText,
        d: TranscriptDelta,
    ) -> TranscriptRevision {
        use ce_core::speech::{HintSource, RecognitionMode, SpeechMeta};
        TranscriptRevision {
            meta: SpeechMeta {
                stream: ce_audio_intel::synth::SYNTH_STREAM,
                utterance: utt,
                sequence_range: (0, 0),
                frame_range: (0, 0),
                time_range: Default::default(),
                input_end_frame: 0,
                discontinuity: None,
                mode: RecognitionMode::Full,
                hints: HintSource::Continuous,
            },
            revision: n,
            status,
            text,
            delta: d,
            commit_amended: false,
            words: None,
            score: None,
            language: None,
            final_reason: None,
        }
    }

    #[test]
    fn assembler_rejects_stale_duplicate_and_late_revisions_and_survives_gaps() {
        let mut u = UtteranceText::new(policy(1, 0, 0));
        let r: Vec<TextRevision> = ["a", "a b", "a b c"]
            .iter()
            .filter_map(|h| u.partial(h, 0))
            .collect();
        let f = u.finalize("a b c d");
        let mk = |t: &TextRevision, s| rev(7, t.revision, s, t.text.clone(), t.delta.clone());
        let mut asm = TranscriptAssembler::new();
        asm.apply(&mk(&r[0], RevisionStatus::Partial)).unwrap();
        assert_eq!(
            asm.apply(&mk(&r[0], RevisionStatus::Partial)),
            Err(Rejected::Stale)
        );
        // Skip r[1]: r[2] is applied from its snapshot.
        asm.apply(&mk(&r[2], RevisionStatus::Partial)).unwrap();
        assert_eq!(asm.gaps(), 1);
        assert_eq!(
            asm.apply(&mk(&r[1], RevisionStatus::Partial)),
            Err(Rejected::Stale),
            "a late older revision never overwrites"
        );
        assert_eq!(asm.text(), &r[2].text);
        asm.apply(&mk(&f, RevisionStatus::Final)).unwrap();
        assert_eq!(asm.text().committed, "a b c d");
        assert_eq!(asm.ended(), Some(RevisionStatus::Final));
        let late = rev(
            7,
            99,
            RevisionStatus::Partial,
            TranscriptText::default(),
            TranscriptDelta::default(),
        );
        assert_eq!(asm.apply(&late), Err(Rejected::Ended));
        let other = rev(
            8,
            0,
            RevisionStatus::Partial,
            TranscriptText::default(),
            TranscriptDelta::default(),
        );
        assert_eq!(asm.apply(&other), Err(Rejected::WrongUtterance));
    }
}
