//! Optional Communication Engine ASR adapter: **sherpa-onnx streaming
//! Zipformer** (the Phase 4 real-ASR baseline, ADR-0023).
//!
//! Implements `ce_speech::AsrFactory` / `AsrAdapter` over the sherpa-onnx
//! C API (crate `sherpa-onnx` 2.0.3, Apache-2.0), which statically links
//! sherpa-onnx and ONNX Runtime. The model is **not** bundled: point
//! [`SherpaConfig`] at a directory holding the INT8 transducer files of
//! `sherpa-onnx-streaming-zipformer-en-2023-06-26` (see
//! `scripts/fetch-model.sh` and README.md).
//!
//! **Declared capabilities (and only these):** streaming, partial results
//! (which may change the last words), endpointing (sherpa-onnx rules),
//! English only, 16 kHz input, deterministic for a fixed configuration.
//! **Not declared:** punctuation and casing (the model emits upper-case
//! words without punctuation), word timestamps (the model reports token
//! start times only; word end times would have to be invented), scores,
//! language reports.
//!
//! Lifecycle: `capabilities()` reads nothing but file sizes and the
//! optional `SHA256SUMS`; `create()` is cheap; `load()` (on the
//! recognition worker thread) creates the shared ONNX sessions once per
//! factory and reuses them while any adapter holds them; each utterance
//! gets its own sherpa-onnx stream.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use ce_core::intel::ScoreSemantics;
use ce_core::speech::{AsrCapabilities, LanguageHint, LanguageSupport, LanguageTag, ModelInfo};
use ce_core::{CeError, ErrorCategory};
use ce_speech::adapter::{AdapterHypothesis, AsrAdapter, AsrFactory, LoadOutcome};
use sherpa_onnx::{
    OnlineModelConfig, OnlineRecognizer, OnlineRecognizerConfig, OnlineStream,
    OnlineTransducerModelConfig,
};

/// Adapter id.
pub const ADAPTER_ID: &str = "ce.asr.sherpa-zipformer";
/// Model id of the approved baseline.
pub const MODEL_ID: &str = "sherpa-onnx-streaming-zipformer-en-2023-06-26";
/// Model input rate.
pub const SAMPLE_RATE: u32 = 16_000;

/// Configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct SherpaConfig {
    /// Directory with the model files.
    pub model_dir: PathBuf,
    /// Encoder file name.
    pub encoder: String,
    /// Decoder file name.
    pub decoder: String,
    /// Joiner file name.
    pub joiner: String,
    /// Tokens file name.
    pub tokens: String,
    /// ONNX Runtime intra-op threads per recognizer (1 keeps it
    /// deterministic and bounded).
    pub num_threads: i32,
    /// Enable sherpa-onnx endpoint rules.
    pub enable_endpoint: bool,
    /// Rule 1: trailing silence (s) ending an utterance with no decoded text.
    pub rule1_min_trailing_silence: f32,
    /// Rule 2: trailing silence (s) after decoded text.
    pub rule2_min_trailing_silence: f32,
    /// Rule 3: maximum utterance length (s).
    pub rule3_min_utterance_length: f32,
    /// Zeros appended before `input_finished` at finalization so the last
    /// chunk is decoded (sherpa-onnx's documented practice), ms.
    pub tail_padding_ms: u32,
}

impl SherpaConfig {
    /// The approved INT8 baseline in `model_dir`.
    pub fn zipformer_en_2023_06_26_int8(model_dir: impl Into<PathBuf>) -> Self {
        Self {
            model_dir: model_dir.into(),
            encoder: "encoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx".into(),
            decoder: "decoder-epoch-99-avg-1-chunk-16-left-128.int8.onnx".into(),
            joiner: "joiner-epoch-99-avg-1-chunk-16-left-128.int8.onnx".into(),
            tokens: "tokens.txt".into(),
            num_threads: 1,
            enable_endpoint: true,
            rule1_min_trailing_silence: 2.4,
            rule2_min_trailing_silence: 1.2,
            rule3_min_utterance_length: 20.0,
            tail_padding_ms: 300,
        }
    }

    fn path(&self, f: &str) -> PathBuf {
        self.model_dir.join(f)
    }

    fn files(&self) -> [PathBuf; 4] {
        [
            self.path(&self.encoder),
            self.path(&self.decoder),
            self.path(&self.joiner),
            self.path(&self.tokens),
        ]
    }

    fn validate(&self) -> Result<(), CeError> {
        if !(1..=16).contains(&self.num_threads) || self.tail_padding_ms > 2_000 {
            return Err(CeError::invalid_config(
                "asr.sherpa_config_invalid",
                "num_threads must be 1..=16 and tail_padding_ms <= 2000",
            ));
        }
        for f in self.files() {
            if !f.is_file() {
                return Err(CeError::new(
                    ErrorCategory::CapabilityResolution,
                    "asr.model_missing",
                    "a speech model file is missing",
                )
                .with_context(
                    "file",
                    f.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                ));
            }
        }
        Ok(())
    }

    fn recognizer_config(&self) -> OnlineRecognizerConfig {
        let s = |p: PathBuf| Some(p.to_string_lossy().into_owned());
        OnlineRecognizerConfig {
            model_config: OnlineModelConfig {
                transducer: OnlineTransducerModelConfig {
                    encoder: s(self.path(&self.encoder)),
                    decoder: s(self.path(&self.decoder)),
                    joiner: s(self.path(&self.joiner)),
                },
                tokens: s(self.path(&self.tokens)),
                num_threads: self.num_threads,
                provider: Some("cpu".into()),
                ..Default::default()
            },
            decoding_method: Some("greedy_search".into()),
            enable_endpoint: self.enable_endpoint,
            rule1_min_trailing_silence: self.rule1_min_trailing_silence,
            rule2_min_trailing_silence: self.rule2_min_trailing_silence,
            rule3_min_utterance_length: self.rule3_min_utterance_length,
            ..Default::default()
        }
    }

    /// A stable fingerprint of the model files: the hash of the directory's
    /// `SHA256SUMS` lines for the configured files when present (no model
    /// bytes are read), else `None`.
    pub fn model_fingerprint(&self) -> Option<String> {
        let sums = std::fs::read_to_string(self.model_dir.join("SHA256SUMS")).ok()?;
        let want = [&self.encoder, &self.decoder, &self.joiner, &self.tokens];
        let mut lines: Vec<&str> = sums
            .lines()
            .filter(|l| want.iter().any(|w| l.ends_with(w.as_str())))
            .collect();
        if lines.len() != want.len() {
            return None;
        }
        lines.sort_unstable();
        Some(format!("sha256sums:{}", fnv(&lines.join("\n"))))
    }
}

fn fnv(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Factory. Adapters created by one factory share one loaded model.
#[derive(Debug, Clone)]
pub struct SherpaFactory {
    cfg: SherpaConfig,
    shared: Arc<Mutex<Weak<Model>>>,
}

struct Model {
    recognizer: OnlineRecognizer,
}

impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Model")
    }
}

impl SherpaFactory {
    /// A factory; nothing is loaded or read here.
    pub fn new(cfg: SherpaConfig) -> Self {
        Self {
            cfg,
            shared: Arc::new(Mutex::new(Weak::new())),
        }
    }

    /// Configuration.
    pub fn config(&self) -> &SherpaConfig {
        &self.cfg
    }
}

impl AsrFactory for SherpaFactory {
    fn capabilities(&self) -> AsrCapabilities {
        let size = self
            .cfg
            .files()
            .iter()
            .filter_map(|f| std::fs::metadata(f).ok().map(|m| m.len()))
            .sum::<u64>();
        AsrCapabilities {
            adapter: ADAPTER_ID.into(),
            version: format!("{}+sherpa-onnx-2.0.3", env!("CARGO_PKG_VERSION")),
            model: Some(ModelInfo {
                id: MODEL_ID.into(),
                version: "int8 epoch-99 avg-1 chunk-16 left-128".into(),
                fingerprint: self.cfg.model_fingerprint(),
                size_bytes: (size > 0).then_some(size),
                license: "Apache-2.0".into(),
            }),
            streaming: true,
            partial_results: true,
            revises_partials: true,
            word_timestamps: false,
            punctuation: false,
            casing: false,
            languages: LanguageSupport::Fixed(vec![LanguageTag::parse("en").expect("valid tag")]),
            reports_language: false,
            endpointing: self.cfg.enable_endpoint,
            sample_rate: SAMPLE_RATE,
            max_utterance_ms: None,
            score_semantics: None::<ScoreSemantics>,
            deterministic: self.cfg.num_threads == 1,
            max_concurrent_streams: None,
            requires_load: true,
        }
    }

    fn create(&self) -> Result<Box<dyn AsrAdapter>, CeError> {
        self.cfg.validate()?;
        Ok(Box::new(SherpaAdapter {
            cfg: self.cfg.clone(),
            shared: self.shared.clone(),
            model: None,
            stream: None,
            last: String::new(),
            endpointed: false,
        }))
    }
}

struct SherpaAdapter {
    cfg: SherpaConfig,
    shared: Arc<Mutex<Weak<Model>>>,
    model: Option<Arc<Model>>,
    stream: Option<OnlineStream>,
    last: String,
    endpointed: bool,
}

impl std::fmt::Debug for SherpaAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SherpaAdapter")
            .field("loaded", &self.model.is_some())
            .finish_non_exhaustive()
    }
}

fn not_loaded() -> CeError {
    CeError::lifecycle("asr.not_loaded", "the speech model is not loaded")
}

impl SherpaAdapter {
    fn model(&self) -> Result<&Model, CeError> {
        self.model.as_deref().ok_or_else(not_loaded)
    }

    /// Decode what is ready and return the current text.
    fn decode(&self) -> Result<(String, bool), CeError> {
        let m = self.model()?;
        let s = self
            .stream
            .as_ref()
            .ok_or_else(|| CeError::lifecycle("asr.no_utterance", "no utterance is open"))?;
        while m.recognizer.is_ready(s) {
            m.recognizer.decode(s);
        }
        let text = m
            .recognizer
            .get_result(s)
            .map(|r| r.text.trim().to_owned())
            .unwrap_or_default();
        let endpoint = self.cfg.enable_endpoint && m.recognizer.is_endpoint(s);
        Ok((text, endpoint))
    }
}

impl AsrAdapter for SherpaAdapter {
    fn load(&mut self) -> Result<LoadOutcome, CeError> {
        if self.model.is_some() {
            return Ok(LoadOutcome { reused: true });
        }
        let mut slot = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(m) = slot.upgrade() {
            self.model = Some(m);
            return Ok(LoadOutcome { reused: true });
        }
        self.cfg.validate()?;
        let recognizer =
            OnlineRecognizer::create(&self.cfg.recognizer_config()).ok_or_else(|| {
                CeError::new(
                    ErrorCategory::Provider,
                    "asr.model_load_failed",
                    "sherpa-onnx could not create the recognizer",
                )
            })?;
        let m = Arc::new(Model { recognizer });
        *slot = Arc::downgrade(&m);
        self.model = Some(m);
        Ok(LoadOutcome { reused: false })
    }

    fn begin(&mut self, _utterance: u64, language: &LanguageHint) -> Result<(), CeError> {
        let unsupported = match language {
            LanguageHint::Fixed(l) => l.primary() != "en",
            LanguageHint::Expected(v) => !v.iter().any(|l| l.primary() == "en"),
            LanguageHint::Auto => false,
        };
        if unsupported {
            return Err(CeError::invalid_input(
                "asr.language_unsupported",
                "this recognizer only supports English",
            ));
        }
        let stream = self.model()?.recognizer.create_stream();
        self.stream = Some(stream);
        self.last.clear();
        self.endpointed = false;
        Ok(())
    }

    fn accept(&mut self, samples: &[f32]) -> Result<Option<AdapterHypothesis>, CeError> {
        {
            let s = self
                .stream
                .as_ref()
                .ok_or_else(|| CeError::lifecycle("asr.no_utterance", "no utterance is open"))?;
            s.accept_waveform(SAMPLE_RATE as i32, samples);
        }
        let (text, endpoint) = self.decode()?;
        let endpoint = endpoint && !self.endpointed;
        self.endpointed |= endpoint;
        if text == self.last && !endpoint {
            return Ok(None);
        }
        self.last.clone_from(&text);
        Ok(Some(AdapterHypothesis {
            text,
            endpoint,
            ..Default::default()
        }))
    }

    fn finalize(&mut self) -> Result<AdapterHypothesis, CeError> {
        {
            let s = self
                .stream
                .as_ref()
                .ok_or_else(|| CeError::lifecycle("asr.no_utterance", "no utterance is open"))?;
            let pad =
                (u64::from(self.cfg.tail_padding_ms) * u64::from(SAMPLE_RATE) / 1_000) as usize;
            if pad > 0 {
                s.accept_waveform(SAMPLE_RATE as i32, &vec![0.0; pad]);
            }
            s.input_finished();
        }
        let (text, _) = self.decode()?;
        self.stream = None;
        self.last.clear();
        Ok(AdapterHypothesis {
            text,
            ..Default::default()
        })
    }

    fn reset(&mut self) {
        self.stream = None;
        self.last.clear();
        self.endpointed = false;
    }
}

/// The model directory from `CE_SHERPA_MODEL_DIR`, if set.
pub fn model_dir_from_env() -> Option<PathBuf> {
    std::env::var_os("CE_SHERPA_MODEL_DIR").map(PathBuf::from)
}

/// Whether `dir` holds the approved INT8 baseline files.
pub fn has_model(dir: &Path) -> bool {
    SherpaConfig::zipformer_en_2023_06_26_int8(dir)
        .validate()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_is_cheap_and_honest_and_missing_models_fail_at_create() {
        let f = SherpaFactory::new(SherpaConfig::zipformer_en_2023_06_26_int8(
            "/nonexistent/model",
        ));
        let c = f.capabilities();
        assert_eq!(c.adapter, ADAPTER_ID);
        assert!(c.streaming && c.partial_results && c.endpointing);
        assert!(!c.punctuation && !c.casing && !c.word_timestamps && !c.reports_language);
        assert!(c.score_semantics.is_none());
        assert_eq!(c.model.as_ref().unwrap().size_bytes, None);
        let Err(e) = f.create() else {
            panic!("create must fail without a model")
        };
        assert_eq!(e.code(), "asr.model_missing");
    }
}
