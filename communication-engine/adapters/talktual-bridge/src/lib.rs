//! Talktual ↔ Communication Engine bridge (ADR-0024).
//!
//! A local WebSocket server that hosts one CE [`Engine`] and lets
//! Talktual's browser client use CE recognition end to end:
//!
//! browser microphone → AudioWorklet → 16-bit PCM over WebSocket →
//! [`push_backend::PushBackend`] (a CE audio backend fed by the network) →
//! CE capture stream (consent gate, negotiation, taps) → Phase 3 analysis
//! (activity segmentation) → Phase 4 recognition (sherpa-onnx) → speech
//! events → JSON over the same WebSocket → Talktual's `AsrCallbacks`.
//!
//! Every WebSocket is one CE session with one capture stream. Nothing is
//! stored. Transcript text crosses the socket only to the client that sent
//! the audio. By default the server binds to 127.0.0.1 and accepts browser
//! connections only from the configured origins.

#![forbid(unsafe_code)]

pub mod protocol;
pub mod push_backend;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ce_core::ErrorCategory;
use ce_core::speech::{
    LanguageDecision, RecognitionConfig, RevisionStatus, SegmentationMode, SpeechEvent,
};
use ce_core::time::{Clock, SystemClock};
use ce_speech::AsrFactory;
use communication_engine::contracts::intel::{ActivityState, AnalysisConfig, AnalysisEvent};
use communication_engine::prelude::*;
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

use protocol::{ClientMsg, PROTOCOL, Segmentation, ServerMsg, decode_pcm};
use push_backend::PushBackend;

/// Bridge settings.
#[derive(Clone, Debug)]
pub struct BridgeConfig {
    /// Browser origins allowed to connect (`Origin` header). Clients that
    /// send no `Origin` (non-browser tools, tests) are allowed.
    pub allowed_origins: Vec<String>,
    /// Longest a graceful stop may take before the session is closed.
    pub stop_timeout: Duration,
    /// Base recognition configuration (segmentation is set per session).
    pub recognition: RecognitionConfig,
    /// Opt-in: append every final (text + wall-clock latencies) to this
    /// JSONL file, for scoring a human test. **Off by default**: it stores
    /// transcript text on disk.
    pub log_finals: Option<std::path::PathBuf>,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            allowed_origins: ["5173", "4173", "8080"]
                .iter()
                .flat_map(|p| {
                    [
                        format!("http://localhost:{p}"),
                        format!("http://127.0.0.1:{p}"),
                    ]
                })
                .collect(),
            stop_timeout: Duration::from_secs(10),
            recognition: RecognitionConfig::default(),
            log_finals: None,
        }
    }
}

/// A running bridge.
pub struct Bridge {
    engine: Engine,
    backend: PushBackend,
    open_lock: tokio::sync::Mutex<()>,
    cfg: BridgeConfig,
    adapter: String,
    model: Option<String>,
    language_ok: Box<dyn Fn(&str) -> bool + Send + Sync>,
    log: Option<std::sync::Mutex<std::fs::File>>,
}

impl std::fmt::Debug for Bridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bridge")
            .field("adapter", &self.adapter)
            .finish_non_exhaustive()
    }
}

impl Bridge {
    /// Build and start an engine with `factory` as the recognizer. Must be
    /// called inside a Tokio runtime.
    pub async fn start(
        factory: impl AsrFactory + 'static,
        cfg: BridgeConfig,
    ) -> Result<Arc<Self>, CeError> {
        let caps = factory.capabilities();
        let languages = match &caps.languages {
            ce_core::speech::LanguageSupport::Fixed(v) => {
                Some(v.iter().map(|t| t.primary().to_owned()).collect::<Vec<_>>())
            }
            ce_core::speech::LanguageSupport::Unknown => None,
        };
        let backend = PushBackend::new();
        let mut config = communication_engine::contracts::config::EngineConfig::default();
        config.speech.max_concurrent_recognitions = 8;
        // Network audio arrives in bursts (jitter, throttled tabs): give the
        // capture ring CE's maximum (5 s) instead of the 250 ms device default.
        config.audio.capture_ring_ms = 5_000;
        let engine = Engine::builder()
            .config(config)
            .audio_backend(Arc::new(backend.clone()))
            .asr_adapter(factory)
            .build()?;
        engine.start().await?;
        let log = match &cfg.log_finals {
            Some(p) => Some(std::sync::Mutex::new(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(p)
                    .map_err(|e| {
                        CeError::new(
                            ErrorCategory::InvalidInput,
                            "bridge.log_unwritable",
                            "cannot open the transcript log",
                        )
                        .with_source(e)
                    })?,
            )),
            None => None,
        };
        Ok(Arc::new(Self {
            log,
            engine,
            backend,
            open_lock: tokio::sync::Mutex::new(()),
            cfg,
            adapter: caps.adapter.clone(),
            model: caps.model.as_ref().map(|m| m.id.clone()),
            language_ok: Box::new(move |tag: &str| {
                let primary = tag.split('-').next().unwrap_or(tag).to_ascii_lowercase();
                languages.as_ref().is_none_or(|v| v.contains(&primary))
            }),
        }))
    }

    /// Accept connections until the listener fails.
    pub async fn serve(self: Arc<Self>, listener: TcpListener) {
        while let Ok((tcp, peer)) = listener.accept().await {
            let me = self.clone();
            tokio::spawn(async move { me.connection(tcp, peer).await });
        }
    }

    /// Shut the engine down.
    pub async fn shutdown(self: Arc<Self>) -> Result<(), CeError> {
        match Arc::try_unwrap(self) {
            Ok(b) => b.engine.shutdown().await.map(|_| ()),
            Err(_) => Ok(()), // connections still hold it; dropping triggers shutdown
        }
    }

    async fn connection(self: Arc<Self>, tcp: TcpStream, _peer: SocketAddr) {
        let allowed = self.cfg.allowed_origins.clone();
        // The callback's signature (and its large error type) is fixed by
        // tungstenite's handshake API.
        #[allow(clippy::result_large_err)]
        let check = move |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
            match req.headers().get("origin").and_then(|o| o.to_str().ok()) {
                Some(o) if !allowed.iter().any(|a| a == o) => {
                    let mut r = ErrorResponse::new(Some("origin not allowed".into()));
                    *r.status_mut() = StatusCode::FORBIDDEN;
                    Err(r)
                }
                _ => Ok(resp),
            }
        };
        let Ok(ws) = tokio_tungstenite::accept_hdr_async(tcp, check).await else {
            return;
        };
        let (mut tx, mut rx) = ws.split();
        let mut session = Session::default();
        let mut events = match self.engine.subscribe() {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut deadline: Option<tokio::time::Instant> = None;
        loop {
            let out: Vec<ServerMsg> = tokio::select! {
                msg = rx.next() => match msg {
                    Some(Ok(Message::Text(t))) => {
                        match serde_json::from_str::<ClientMsg>(&t) {
                            Ok(ClientMsg::Start { lang, sample_rate, segmentation }) => {
                                self.begin(&mut session, &lang, sample_rate, segmentation).await
                            }
                            Ok(ClientMsg::Stop) => {
                                if let Some(id) = session.stream {
                                    session.stop_at = Some(std::time::Instant::now());
                                    self.drain(&session).await;
                                    let _ = self.engine.close_audio(id).await;
                                    deadline = Some(tokio::time::Instant::now() + self.cfg.stop_timeout);
                                    Vec::new()
                                } else {
                                    vec![ServerMsg::End { aborted: false }]
                                }
                            }
                            Ok(ClientMsg::Abort) => {
                                self.end(&mut session).await;
                                vec![ServerMsg::End { aborted: true }]
                            }
                            Err(_) => vec![error("bridge.bad_message", "unrecognized message", false)],
                        }
                    }
                    Some(Ok(Message::Binary(b))) => {
                        if let Some(slot) = &session.slot {
                            if !slot.push_i16(&decode_pcm(&b)) && !session.warned_closed {
                                session.warned_closed = true;
                                vec![ServerMsg::Warning { kind: "audio_after_stop".into() }]
                            } else {
                                Vec::new()
                            }
                        } else {
                            Vec::new()
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                        self.end(&mut session).await;
                        return;
                    }
                    Some(Ok(_)) => Vec::new(),
                },
                ev = events.recv(), if session.stream.is_some() => match ev {
                    Ok(env) => self.translate(&mut session, &env),
                    Err(SubscriptionError::Lagged { .. }) => vec![ServerMsg::Warning { kind: "events_lagged".into() }],
                    Err(SubscriptionError::Closed) => vec![error("bridge.engine_stopped", "the engine stopped", true)],
                },
                () = async {
                    match deadline {
                        Some(d) => tokio::time::sleep_until(d).await,
                        None => std::future::pending().await,
                    }
                } => {
                    deadline = None;
                    self.end(&mut session).await;
                    vec![error("bridge.stop_timeout", "recognition did not finish in time", false), ServerMsg::End { aborted: false }]
                }
            };
            for m in out {
                let ended = matches!(m, ServerMsg::End { .. });
                let text = serde_json::to_string(&m).unwrap_or_default();
                if tx.send(Message::Text(text.into())).await.is_err() {
                    self.end(&mut session).await;
                    return;
                }
                if ended {
                    deadline = None;
                    self.end(&mut session).await;
                }
            }
        }
    }

    async fn begin(
        &self,
        s: &mut Session,
        lang: &str,
        sample_rate: u32,
        segmentation: Segmentation,
    ) -> Vec<ServerMsg> {
        if s.stream.is_some() {
            return vec![error(
                "bridge.already_started",
                "this connection is already recognizing",
                false,
            )];
        }
        if !(self.language_ok)(lang) {
            return vec![error(
                "language-unsupported",
                "the speech model does not support this language",
                true,
            )];
        }
        if !(8_000..=96_000).contains(&sample_rate) {
            return vec![error(
                "bridge.bad_rate",
                "sample rate must be 8000..=96000",
                true,
            )];
        }
        s.rate = sample_rate;
        match self.open(s, sample_rate, segmentation).await {
            Ok(()) => vec![ServerMsg::Ready {
                protocol: PROTOCOL,
                adapter: self.adapter.clone(),
                model: self.model.clone(),
                segmentation,
            }],
            Err(e) => {
                self.end(s).await;
                vec![error(e.code(), &e.to_string(), true)]
            }
        }
    }

    async fn open(&self, s: &mut Session, rate: u32, seg: Segmentation) -> Result<(), CeError> {
        let info = self.engine.create_session(SessionConfig::default()).await?;
        s.session = Some(info.id);
        // The browser user consented to this route in Talktual before the
        // microphone was opened; CE's own gate is satisfied for this session.
        self.engine
            .update_consent(
                info.id,
                ConsentPurpose::Capture,
                ConsentState::Granted {
                    at: SystemClock::new().wall(),
                },
            )
            .await?;
        let request = StreamRequest {
            format: FormatPreference {
                sample_rate: Some(rate),
                channels: Some(1),
                sample_format: Some(SampleFormat::I16),
                buffer_frames: None,
            },
            ..StreamRequest::capture()
        };
        let (mut reader, slot) = {
            let _g = self.open_lock.lock().await;
            let reader = self.engine.open_capture(info.id, request).await?;
            (reader, self.backend.take_last_slot())
        };
        let id = reader.info().id;
        s.stream = Some(id);
        s.slot = slot;
        // Nobody in the bridge needs the host copy of the audio: drain it.
        tokio::spawn(async move { while reader.recv().await.is_ok() {} });
        self.engine.start_audio(id).await?;
        let mut rcfg = self.cfg.recognition.clone();
        // Absorb network bursts (10 s of 10 ms blocks) rather than drop.
        rcfg.budget.queue_blocks = rcfg.budget.queue_blocks.max(1_000);
        match seg {
            Segmentation::Continuous => rcfg.segmentation = SegmentationMode::Continuous,
            Segmentation::Activity => {
                rcfg.segmentation = SegmentationMode::ActivityHints;
                let mut acfg = AnalysisConfig::default();
                acfg.budget.degrade = false;
                self.engine.start_analysis(id, Some(acfg)).await?;
            }
        }
        self.engine.start_recognition(id, Some(rcfg)).await?;
        Ok(())
    }

    /// Map an engine event to client messages.
    fn translate(&self, s: &mut Session, env: &EventEnvelope) -> Vec<ServerMsg> {
        let Some(stream) = s.stream else {
            return Vec::new();
        };
        match &env.payload {
            EventPayload::Analysis(AnalysisEvent::ActivityChanged { meta, state, .. })
                if meta.stream == stream =>
            {
                vec![ServerMsg::Activity {
                    active: *state == ActivityState::Active,
                }]
            }
            EventPayload::Speech(e) if e.stream() == stream => match e {
                SpeechEvent::UtteranceStarted { meta } => {
                    s.utterance_at = Some(std::time::Instant::now());
                    s.first_partial_ms = None;
                    vec![ServerMsg::Utterance {
                        utterance: meta.utterance,
                    }]
                }
                SpeechEvent::Transcript(r) => {
                    let utterance = r.meta.utterance;
                    let reason = r
                        .final_reason
                        .map(|x| format!("{x:?}").to_lowercase())
                        .unwrap_or_default();
                    if r.status == RevisionStatus::Partial && s.first_partial_ms.is_none() {
                        s.first_partial_ms = s.utterance_at.map(|t| t.elapsed().as_millis() as u64);
                    }
                    if r.status == RevisionStatus::Final {
                        self.log_final(s, r);
                    }
                    match r.status {
                        RevisionStatus::Partial => vec![ServerMsg::Partial {
                            utterance,
                            revision: r.revision,
                            committed: r.text.committed.clone(),
                            provisional: r.text.provisional.clone(),
                        }],
                        RevisionStatus::Final => {
                            let (f0, f1) = r.meta.frame_range;
                            let rate = u64::from(s.rate.max(1));
                            vec![ServerMsg::Final {
                                utterance,
                                text: r.text.committed.clone(),
                                reason,
                                language: r.language.as_ref().and_then(|l| match &l.decision {
                                    LanguageDecision::Identified { language } => {
                                        Some(language.to_string())
                                    }
                                    LanguageDecision::Abstained { .. } => None,
                                }),
                                duration_ms: (f1 - f0) * 1_000 / rate,
                            }]
                        }
                        RevisionStatus::Cancelled => {
                            vec![ServerMsg::Cancelled { utterance, reason }]
                        }
                    }
                }
                SpeechEvent::Failed { error: err, .. } => {
                    vec![error(&err.code, &err.message, true)]
                }
                SpeechEvent::QueuePressure { .. } => vec![ServerMsg::Warning {
                    kind: "recognizer_behind".into(),
                }],
                SpeechEvent::Stopped { .. } => vec![ServerMsg::End { aborted: false }],
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// Append a final to the opt-in transcript log.
    fn log_final(&self, s: &Session, r: &ce_core::speech::TranscriptRevision) {
        let Some(log) = &self.log else {
            return;
        };
        let (f0, f1) = r.meta.frame_range;
        let line = serde_json::json!({
            "unixMs": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            "utterance": r.meta.utterance,
            "text": r.text.committed,
            "reason": r.final_reason.map(|x| format!("{x:?}").to_lowercase()),
            "audioMs": (f1 - f0) * 1_000 / u64::from(s.rate.max(1)),
            "firstPartialAfterUtteranceStartMs": s.first_partial_ms,
            "finalAfterReleaseMs": s.stop_at.map(|t| t.elapsed().as_millis() as u64),
            "adapter": self.adapter,
        });
        use std::io::Write;
        if let Ok(mut f) = log.lock() {
            let _ = writeln!(f, "{line}");
        }
    }

    /// Wait (bounded, 2 s) until CE's capture worker has taken every frame
    /// pushed so far out of the ring, so a graceful stop loses nothing.
    async fn drain(&self, s: &Session) {
        let (Some(id), Some(slot)) = (s.stream, s.slot.as_ref()) else {
            return;
        };
        let pushed = slot.pushed_frames();
        let until = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < until {
            let consumed = self
                .engine
                .audio_streams()
                .await
                .ok()
                .and_then(|v| v.into_iter().find(|st| st.info.id == id))
                .map(|st| st.counters.frames + st.counters.ring_overflow_frames);
            match consumed {
                Some(c) if c + 512 >= pushed => return, // < one block may stay partial
                Some(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                None => return,
            }
        }
    }

    async fn end(&self, s: &mut Session) {
        s.slot = None;
        s.stream = None;
        if let Some(id) = s.session.take() {
            let _ = self.engine.close_session(id).await;
        }
    }
}

#[derive(Default)]
struct Session {
    session: Option<SessionId>,
    stream: Option<AudioStreamId>,
    slot: Option<push_backend::PushSlot>,
    rate: u32,
    warned_closed: bool,
    utterance_at: Option<std::time::Instant>,
    first_partial_ms: Option<u64>,
    stop_at: Option<std::time::Instant>,
}

fn error(code: &str, message: &str, fatal: bool) -> ServerMsg {
    ServerMsg::Error {
        code: code.into(),
        message: message.into(),
        fatal,
    }
}
