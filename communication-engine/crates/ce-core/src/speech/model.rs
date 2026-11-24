//! Speech recognition and language results, metadata, events and status.
//!
//! Positions are frames of the capture stream's sample clock and times are
//! CE monotonic nanoseconds on the Phase 2 block `timestamp` timeline — the
//! same timeline as Phase 3 results, so the two can be joined by frame.
//! Timestamps are never shifted or invented: word timings appear only when
//! the adapter provides them, mapped exactly from adapter frames.

use serde::{Deserialize, Serialize};

use crate::error::{CeError, ErrorReport};
use crate::ids::AudioStreamId;
use crate::intel::ScoreSemantics;
use crate::intel::model::DiscontinuityContext;
use crate::time::MonotonicTime;

/// A validated BCP-47-style language tag (`en`, `pt-BR`, `zh-Hant`):
/// a 2–3 letter primary subtag, then up to three 2–8 character
/// alphanumeric subtags. Stored in canonical case (primary lowercase).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LanguageTag(String);

impl LanguageTag {
    /// Parse and canonicalize.
    pub fn parse(s: &str) -> Result<Self, CeError> {
        let bad = || {
            CeError::invalid_input("speech.bad_language_tag", "not a valid language tag")
                .with_context("tag", s.chars().take(32).collect::<String>())
        };
        let mut parts = s.split('-');
        let primary = parts.next().ok_or_else(bad)?;
        if !(2..=3).contains(&primary.len()) || !primary.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(bad());
        }
        let mut out = primary.to_ascii_lowercase();
        let mut n = 0;
        for p in parts {
            n += 1;
            if n > 3 || !(2..=8).contains(&p.len()) || !p.chars().all(|c| c.is_ascii_alphanumeric())
            {
                return Err(bad());
            }
            out.push('-');
            out.push_str(&match p.len() {
                2 => p.to_ascii_uppercase(),
                4 => {
                    let mut c = p.to_ascii_lowercase();
                    c[..1].make_ascii_uppercase();
                    c
                }
                _ => p.to_ascii_lowercase(),
            });
        }
        Ok(Self(out))
    }

    /// The tag.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The primary language subtag.
    pub fn primary(&self) -> &str {
        self.0.split('-').next().unwrap_or(&self.0)
    }
}

impl TryFrom<String> for LanguageTag {
    type Error = CeError;
    fn try_from(s: String) -> Result<Self, CeError> {
        Self::parse(&s)
    }
}

impl From<LanguageTag> for String {
    fn from(t: LanguageTag) -> String {
        t.0
    }
}

impl std::fmt::Display for LanguageTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which languages an adapter can recognize.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "languages")]
pub enum LanguageSupport {
    /// Exactly these.
    Fixed(Vec<LanguageTag>),
    /// The adapter does not say (treat as unknown, never as "all").
    Unknown,
}

/// A model behind an adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model id.
    pub id: String,
    /// Model version.
    pub version: String,
    /// Content fingerprint (e.g. a hash of the weights), when known.
    pub fingerprint: Option<String>,
    /// Size on disk, when known.
    pub size_bytes: Option<u64>,
    /// License of the model.
    pub license: String,
}

/// What an ASR adapter can and cannot do. Discovery is cheap: building
/// this must not load a model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AsrCapabilities {
    /// Stable adapter id, e.g. `ce.asr.scripted`.
    pub adapter: String,
    /// Adapter version.
    pub version: String,
    /// The model, if any.
    pub model: Option<ModelInfo>,
    /// Produces results while audio is still arriving.
    pub streaming: bool,
    /// Produces partial (provisional) hypotheses.
    pub partial_results: bool,
    /// Partial hypotheses may change earlier words.
    pub revises_partials: bool,
    /// Per-word timings.
    pub word_timestamps: bool,
    /// Punctuation in output.
    pub punctuation: bool,
    /// Letter case in output.
    pub casing: bool,
    /// Recognizable languages.
    pub languages: LanguageSupport,
    /// Reports the language it recognized.
    pub reports_language: bool,
    /// Detects end of speech itself.
    pub endpointing: bool,
    /// Required input rate (mono f32); CE resamples to it.
    pub sample_rate: u32,
    /// Largest audio window per utterance it accepts, if limited, ms.
    pub max_utterance_ms: Option<u32>,
    /// Semantics of the scores it reports (`None`: no scores).
    pub score_semantics: Option<ScoreSemantics>,
    /// Same input and configuration always give the same output.
    pub deterministic: bool,
    /// Concurrent streams one loaded model supports (`None`: no limit).
    pub max_concurrent_streams: Option<u32>,
    /// Needs an (expensive) load before use.
    pub requires_load: bool,
}

/// Identity of a recognizer in results.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecognizerRef {
    /// Adapter id.
    pub adapter: String,
    /// Adapter version.
    pub version: String,
    /// Model fingerprint, when known.
    pub model_fingerprint: Option<String>,
    /// `RecognitionConfig::fingerprint`.
    pub config_fingerprint: String,
}

/// Recognition operating level (load-driven).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecognitionMode {
    /// Everything enabled.
    Full,
    /// Partials suppressed and chunks coalesced ×4; finals unaffected.
    Throttled,
}

/// What delimited the utterance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HintSource {
    /// Phase 3 activity and segment events.
    Activity,
    /// Continuous feeding (configured, or Phase 3 unavailable/shedding).
    Continuous,
}

/// Metadata of every transcript result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeechMeta {
    /// Stream.
    pub stream: AudioStreamId,
    /// Utterance number within the stream (0-based, never reused).
    pub utterance: u64,
    /// Source block sequences that contributed [first, last].
    pub sequence_range: (u64, u64),
    /// Audio of the utterance [start, end) on the stream's sample clock.
    pub frame_range: (u64, u64),
    /// Same, as monotonic time.
    pub time_range: (MonotonicTime, MonotonicTime),
    /// Last input frame the recognizer had consumed when this result was
    /// produced (for latency measurement in audio time).
    pub input_end_frame: u64,
    /// A discontinuity that ended or shaped the utterance.
    pub discontinuity: Option<DiscontinuityContext>,
    /// Operating level.
    pub mode: RecognitionMode,
    /// Delimitation source.
    pub hints: HintSource,
}

/// Status of a revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionStatus {
    /// More revisions follow.
    Partial,
    /// Last revision of the utterance; `committed` is the final text.
    Final,
    /// Last revision; the utterance was abandoned (reset, stop,
    /// failure): `committed` keeps what was committed, nothing else follows.
    Cancelled,
}

/// Why an utterance ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalReason {
    /// Phase 3 segment ended on its silence policy.
    SegmentEnd,
    /// Phase 3 endpoint candidate reached the configured tier.
    EndpointCandidate,
    /// The adapter detected the end of speech.
    AdapterEndpoint,
    /// `max_utterance_ms` (or the adapter's maximum) was reached.
    MaxDuration,
    /// A gap, restart or format change; utterances never span one.
    Discontinuity,
    /// The capture stream ended.
    StreamEnd,
    /// Recognition was stopped.
    Stopped,
    /// Recognition was reset.
    Reset,
    /// The adapter failed.
    Failure,
}

/// Snapshot of an utterance's text. `committed` only grows within an
/// utterance (append-only), except when a final amends it
/// (`commit_amended`); `provisional` may change arbitrarily.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TranscriptText {
    /// Stable text.
    pub committed: String,
    /// Volatile tail (empty in finals).
    pub provisional: String,
}

impl TranscriptText {
    /// `committed` and `provisional` joined with one space.
    pub fn full(&self) -> String {
        match (self.committed.is_empty(), self.provisional.is_empty()) {
            (_, true) => self.committed.clone(),
            (true, false) => self.provisional.clone(),
            (false, false) => format!("{} {}", self.committed, self.provisional),
        }
    }
}

/// How to get from the previous revision's snapshot to this one: keep the
/// first `keep_committed_chars` characters (Unicode scalar values) of the
/// previous committed text, append `append_committed`, then replace the
/// provisional text with `provisional`. Applying deltas in revision order
/// from revision 0 reproduces every snapshot exactly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TranscriptDelta {
    /// Characters of the previous committed text kept.
    pub keep_committed_chars: u32,
    /// Appended to the committed text.
    pub append_committed: String,
    /// The new provisional text.
    pub provisional: String,
}

/// Per-word timing, mapped exactly from the adapter's frame positions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WordTiming {
    /// Word.
    pub text: String,
    /// [start, end) frames on the stream's sample clock.
    pub frame_range: (u64, u64),
    /// Monotonic time range.
    pub time_range: (MonotonicTime, MonotonicTime),
    /// Adapter score (see the recognizer's score semantics).
    pub score: Option<f32>,
    /// Inside the committed text.
    pub committed: bool,
}

/// A score with its meaning attached.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Score {
    /// Value.
    pub value: f32,
    /// Semantics (never a probability unless calibrated).
    pub semantics: ScoreSemantics,
}

/// One revision of one utterance. Revisions of an utterance are numbered
/// 0, 1, 2, … without gaps; a consumer must ignore a revision whose number
/// is not greater than the last it applied (stale or duplicate) and must
/// use the snapshot (not the delta) after a gap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranscriptRevision {
    /// Metadata.
    pub meta: SpeechMeta,
    /// Revision number within the utterance.
    pub revision: u32,
    /// Status.
    pub status: RevisionStatus,
    /// Full snapshot (authoritative).
    pub text: TranscriptText,
    /// Change from revision − 1 (from empty for revision 0).
    pub delta: TranscriptDelta,
    /// A final changed text that had been committed.
    pub commit_amended: bool,
    /// Word timings, when the adapter provides them and they are enabled.
    pub words: Option<Vec<WordTiming>>,
    /// Utterance score, when the adapter provides one.
    pub score: Option<Score>,
    /// Language observation for the utterance (finals only).
    pub language: Option<LanguageObservation>,
    /// Why it ended (finals and cancellations).
    pub final_reason: Option<FinalReason>,
}

/// What a language observation is about. Session, utterance and span
/// languages are distinct and never substituted for one another.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "scope")]
pub enum LanguageScope {
    /// One utterance's final text.
    Utterance {
        /// Utterance.
        utterance: u64,
    },
    /// A character span of an utterance (code-switching). **Not produced
    /// in Phase 4** (see `SpeechCapabilities::code_switching`); reserved.
    Span {
        /// Utterance.
        utterance: u64,
        /// Character range [start, end).
        chars: (u32, u32),
    },
    /// Aggregate over the stream's decided utterances so far.
    Stream,
}

/// Where a language observation came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "source")]
pub enum LanguageSource {
    /// Declared by configuration (`LanguageHint::Fixed`): not inferred.
    Declared,
    /// Reported by the recognizer.
    Recognizer {
        /// Adapter id.
        adapter: String,
    },
    /// A text language identifier.
    TextIdentifier {
        /// Identifier id.
        id: String,
        /// Version.
        version: String,
    },
    /// Aggregated from utterance observations.
    Aggregate,
}

/// Why no language was decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbstainReason {
    /// Too little text (below `min_letters`) or no decided utterances.
    InsufficientEvidence,
    /// Score below `min_score`.
    LowConfidence,
    /// Evidence split between languages (e.g. no session majority).
    Ambiguous,
    /// The identifier's answer is outside the expected set or unsupported.
    Unsupported,
}

/// The decision part of an observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum LanguageDecision {
    /// A language was decided.
    Identified {
        /// Language.
        language: LanguageTag,
    },
    /// No decision.
    Abstained {
        /// Why.
        reason: AbstainReason,
    },
}

/// A ranked candidate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LanguageCandidate {
    /// Language.
    pub language: LanguageTag,
    /// Score (see `score_semantics`), if the source reports one.
    pub score: Option<f32>,
}

/// A language observation with provenance. Describes the **language of the
/// text**, never anything about the person (nationality, ethnicity,
/// location, identity).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LanguageObservation {
    /// Scope.
    pub scope: LanguageScope,
    /// Decision.
    pub decision: LanguageDecision,
    /// Candidates, best first (may be one or empty).
    pub candidates: Vec<LanguageCandidate>,
    /// Meaning of candidate scores (`None`: no scores).
    pub score_semantics: Option<ScoreSemantics>,
    /// Source.
    pub source: LanguageSource,
    /// Letters of evidence (utterance) or decided utterances (stream).
    pub evidence: u32,
    /// Fingerprint of the identifier configuration.
    pub fingerprint: String,
}

/// What the speech layer supports.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeechCapabilities {
    /// The recognizer.
    pub recognizer: AsrCapabilities,
    /// Text language identifier id and version, if available.
    pub language_identifier: Option<(String, String)>,
    /// Languages the identifier can name.
    pub identifiable_languages: Vec<LanguageTag>,
    /// Intra-utterance language spans. **false**: not supported (see the
    /// Phase 4 evaluation).
    pub code_switching: bool,
}

/// Why recognition ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecognitionEndReason {
    /// Stopped by request.
    Stopped,
    /// The capture stream was closed.
    StreamClosed,
    /// Capture authorization was withdrawn; queued audio was discarded.
    Revoked,
    /// The stream was invalidated.
    StreamInvalidated,
    /// The stream failed.
    StreamFailed,
    /// Model load failed or timed out.
    LoadFailed,
    /// The worker panicked or stalled.
    Failed,
    /// The runtime shut down.
    Shutdown,
}

/// Why the recognizer was reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetReason {
    /// Requested by the host.
    Requested,
    /// A discontinuity.
    Discontinuity,
    /// The adapter reported an error and was reset.
    AdapterError,
}

/// Latency summary in ms (p50, p95, p99 over observed values).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Percentiles {
    /// Observations.
    pub count: u64,
    /// Median.
    pub p50: f32,
    /// 95th percentile.
    pub p95: f32,
    /// 99th percentile.
    pub p99: f32,
    /// Maximum.
    pub max: f32,
}

/// Counters and latency summaries for one recognition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct RecognitionCounters {
    /// Blocks consumed.
    pub blocks: u64,
    /// Audio fed to the adapter, ms.
    pub audio_ms: u64,
    /// Utterances opened.
    pub utterances: u64,
    /// Revisions emitted (all statuses).
    pub revisions: u64,
    /// Finals.
    pub finals: u64,
    /// Cancelled utterances.
    pub cancelled: u64,
    /// Finals that amended committed text.
    pub commit_amendments: u64,
    /// Blocks dropped by the tap (recognizer behind).
    pub tap_dropped_blocks: u64,
    /// Phase 3 hints dropped (hint queue full).
    pub hint_dropped: u64,
    /// Discontinuities handled.
    pub discontinuities: u64,
    /// Adapter errors (each followed by a reset).
    pub adapter_errors: u64,
    /// Malformed adapter outputs rejected.
    pub malformed_outputs: u64,
    /// Highest load seen, permille.
    pub max_load_permille: u32,
    /// Highest tap backlog seen.
    pub max_backlog: u32,
    /// Model load time, ms.
    pub load_ms: Option<u32>,
    /// Wall-clock processing time per block, ms.
    pub processing_ms: Percentiles,
    /// Audio time from utterance start to its first partial, ms.
    pub first_partial_ms: Percentiles,
    /// Audio time from utterance end to its final, ms.
    pub final_delay_ms: Percentiles,
}

/// Snapshot of one recognition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecognitionStatus {
    /// Stream.
    pub stream: AudioStreamId,
    /// Recognizer identity.
    pub recognizer: RecognizerRef,
    /// Capabilities.
    pub capabilities: SpeechCapabilities,
    /// Running.
    pub running: bool,
    /// Model loaded.
    pub loaded: bool,
    /// Operating level.
    pub mode: RecognitionMode,
    /// Delimitation in force.
    pub hints: HintSource,
    /// Open utterance, if any.
    pub open_utterance: Option<u64>,
    /// Stream-scope language observation.
    pub stream_language: Option<LanguageObservation>,
    /// Counters.
    pub counters: RecognitionCounters,
    /// Why it ended.
    pub ended: Option<RecognitionEndReason>,
}

/// Speech events (`EventPayload::Speech`). Transcript-bearing events are
/// classified `Content`; language observations `Personal`; the rest
/// `Operational`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "speech_event")]
#[non_exhaustive]
pub enum SpeechEvent {
    /// Recognition began (model may still be loading).
    Started {
        /// Stream.
        stream: AudioStreamId,
        /// Recognizer.
        recognizer: RecognizerRef,
        /// Capabilities.
        capabilities: Box<SpeechCapabilities>,
    },
    /// The model is loaded and warmed up.
    ModelReady {
        /// Stream.
        stream: AudioStreamId,
        /// Load time, ms.
        load_ms: u32,
        /// Whether an already-loaded model was reused.
        reused: bool,
    },
    /// An utterance opened.
    UtteranceStarted {
        /// Metadata.
        meta: SpeechMeta,
    },
    /// A transcript revision.
    Transcript(Box<TranscriptRevision>),
    /// Stream-scope language observation changed.
    LanguageObserved {
        /// Stream.
        stream: AudioStreamId,
        /// Observation.
        observation: LanguageObservation,
    },
    /// The recognizer was reset.
    Reset {
        /// Stream.
        stream: AudioStreamId,
        /// Why.
        reason: ResetReason,
    },
    /// A discontinuity was handled.
    Discontinuity {
        /// Stream.
        stream: AudioStreamId,
        /// What happened.
        context: DiscontinuityContext,
        /// Where audio resumed.
        resume_frame: u64,
    },
    /// The recognizer fell behind: blocks were dropped or the backlog is high.
    QueuePressure {
        /// Stream.
        stream: AudioStreamId,
        /// Backlog, blocks.
        backlog: u32,
        /// Total blocks dropped so far.
        dropped_total: u64,
    },
    /// The operating level changed.
    ModeChanged {
        /// Stream.
        stream: AudioStreamId,
        /// Previous.
        from: RecognitionMode,
        /// New.
        to: RecognitionMode,
        /// Load, permille.
        load_permille: u32,
    },
    /// Delimitation fell back to (or recovered from) continuous mode.
    HintsChanged {
        /// Stream.
        stream: AudioStreamId,
        /// Now in force.
        hints: HintSource,
    },
    /// The recognition failed.
    Failed {
        /// Stream.
        stream: AudioStreamId,
        /// Redacted error.
        error: ErrorReport,
    },
    /// Recognition ended.
    Stopped {
        /// Stream.
        stream: AudioStreamId,
        /// Why.
        reason: RecognitionEndReason,
        /// Final counters.
        counters: Box<RecognitionCounters>,
    },
}

impl SpeechEvent {
    /// Stable kind name.
    pub fn kind(&self) -> &'static str {
        match self {
            SpeechEvent::Started { .. } => "started",
            SpeechEvent::ModelReady { .. } => "model_ready",
            SpeechEvent::UtteranceStarted { .. } => "utterance_started",
            SpeechEvent::Transcript(_) => "transcript",
            SpeechEvent::LanguageObserved { .. } => "language_observed",
            SpeechEvent::Reset { .. } => "reset",
            SpeechEvent::Discontinuity { .. } => "discontinuity",
            SpeechEvent::QueuePressure { .. } => "queue_pressure",
            SpeechEvent::ModeChanged { .. } => "mode_changed",
            SpeechEvent::HintsChanged { .. } => "hints_changed",
            SpeechEvent::Failed { .. } => "failed",
            SpeechEvent::Stopped { .. } => "stopped",
        }
    }

    /// The stream.
    pub fn stream(&self) -> AudioStreamId {
        match self {
            SpeechEvent::Started { stream, .. }
            | SpeechEvent::ModelReady { stream, .. }
            | SpeechEvent::LanguageObserved { stream, .. }
            | SpeechEvent::Reset { stream, .. }
            | SpeechEvent::Discontinuity { stream, .. }
            | SpeechEvent::QueuePressure { stream, .. }
            | SpeechEvent::ModeChanged { stream, .. }
            | SpeechEvent::HintsChanged { stream, .. }
            | SpeechEvent::Failed { stream, .. }
            | SpeechEvent::Stopped { stream, .. } => *stream,
            SpeechEvent::UtteranceStarted { meta } => meta.stream,
            SpeechEvent::Transcript(r) => r.meta.stream,
        }
    }

    /// Privacy class: transcripts are `Content`, language observations
    /// `Personal`, everything else `Operational`.
    pub fn data_class(&self) -> crate::privacy::DataClass {
        use crate::privacy::DataClass;
        match self {
            SpeechEvent::Transcript(_) => DataClass::Content,
            SpeechEvent::LanguageObserved { .. } => DataClass::Personal,
            _ => DataClass::Operational,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_tags_validate_and_canonicalize() {
        assert_eq!(LanguageTag::parse("EN").unwrap().as_str(), "en");
        assert_eq!(LanguageTag::parse("pt-br").unwrap().as_str(), "pt-BR");
        assert_eq!(
            LanguageTag::parse("zh-hant-tw").unwrap().as_str(),
            "zh-Hant-TW"
        );
        assert_eq!(LanguageTag::parse("es-419").unwrap().primary(), "es");
        for bad in [
            "",
            "e",
            "english",
            "en-",
            "en-x",
            "1a",
            "en-US-a-b-c",
            "en us",
        ] {
            assert!(LanguageTag::parse(bad).is_err(), "{bad}");
        }
        let json = serde_json::to_string(&LanguageTag::parse("fr").unwrap()).unwrap();
        assert_eq!(json, "\"fr\"");
        assert!(serde_json::from_str::<LanguageTag>("\"not a tag\"").is_err());
    }

    #[test]
    fn transcript_events_serialize_with_tags_and_classes() {
        let stream = AudioStreamId::from_uuid(uuid::Uuid::from_u128(9));
        let meta = SpeechMeta {
            stream,
            utterance: 3,
            sequence_range: (1, 9),
            frame_range: (160, 1600),
            time_range: (MonotonicTime::from_nanos(10), MonotonicTime::from_nanos(90)),
            input_end_frame: 1600,
            discontinuity: None,
            mode: RecognitionMode::Full,
            hints: HintSource::Activity,
        };
        let rev = TranscriptRevision {
            meta,
            revision: 2,
            status: RevisionStatus::Final,
            text: TranscriptText {
                committed: "hello there".into(),
                provisional: String::new(),
            },
            delta: TranscriptDelta {
                keep_committed_chars: 5,
                append_committed: " there".into(),
                provisional: String::new(),
            },
            commit_amended: false,
            words: None,
            score: None,
            language: Some(LanguageObservation {
                scope: LanguageScope::Utterance { utterance: 3 },
                decision: LanguageDecision::Abstained {
                    reason: AbstainReason::InsufficientEvidence,
                },
                candidates: vec![],
                score_semantics: None,
                source: LanguageSource::Declared,
                evidence: 10,
                fingerprint: "x".into(),
            }),
            final_reason: Some(FinalReason::SegmentEnd),
        };
        let e = SpeechEvent::Transcript(Box::new(rev));
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["speech_event"], "transcript");
        assert_eq!(v["status"], "final");
        assert_eq!(v["language"]["scope"]["scope"], "utterance");
        assert_eq!(v["language"]["decision"]["decision"], "abstained");
        let back: SpeechEvent = serde_json::from_value(v).unwrap();
        assert_eq!(back, e);
        assert_eq!(back.stream(), stream);
        assert_eq!(back.data_class(), crate::privacy::DataClass::Content);
        assert_eq!(
            TranscriptText {
                committed: "a".into(),
                provisional: "b".into()
            }
            .full(),
            "a b"
        );
    }
}
