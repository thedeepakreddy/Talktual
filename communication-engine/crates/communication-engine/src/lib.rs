//! # communication-engine — runtime and public facade
//!
//! The facade is the only supported way to run the Communication Engine:
//!
//! ```no_run
//! use communication_engine::{Engine, contracts::session::SessionConfig};
//!
//! # async fn demo(factory: impl communication_engine::contracts::module::ModuleFactory)
//! # -> Result<(), communication_engine::contracts::CeError> {
//! let engine = Engine::builder().build()?;          // inside a Tokio runtime
//! let mut events = engine.subscribe()?;               // observe from now on
//! engine.register_module(factory).await?;             // no instantiation yet
//! let caps = engine.capabilities().await?;            // discovery
//! engine.start().await?;                              // init + start modules
//! let session = engine.create_session(SessionConfig::default()).await?;
//! let first = events.recv().await;                    // correlated events
//! let report = engine.shutdown().await?;              // graceful, bounded
//! # let _ = (caps, session, first, report); Ok(()) }
//! ```
//!
//! ## API status
//!
//! **Provisional** ([`FACADE_API_VERSION`] `0.1`). The facade is versioned and
//! intentionally small, but CE has not published a stable SDK; expect changes.
//!
//! ## Async runtime
//!
//! The engine runs on the caller's **Tokio** runtime (multi-thread or
//! current-thread) and never creates one itself. It spawns exactly one core
//! task plus one task per started module; all are joined or aborted at
//! shutdown. See ADR-0002.
//!
//! ## Boundaries
//!
//! The facade has no knowledge of any UI, product, database, transport, cloud
//! vendor or speech provider.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod runtime;
#[cfg(feature = "tracing")]
mod tracing_sink;

use std::sync::Arc;
use std::sync::atomic::Ordering;

pub use ce_audio as audio;
use ce_audio::{AudioBackend, CaptureReader, PlaybackWriter, StreamEndpoint};
pub use ce_audio_intel as audio_intel;
pub use ce_core as contracts;
use ce_core::CeResult;
use ce_core::audio::{AudioDirection, AudioStreamStatus, DeviceCatalog, StreamRequest};
use ce_core::capability::CapabilityInfo;
use ce_core::command::{Command, CommandEnvelope, CommandOutcome, ShutdownReport};
use ce_core::config::EngineConfig;
use ce_core::error::{CeError, ErrorCategory};
use ce_core::event::EventEnvelope;
use ce_core::health::{MetricsSnapshot, RuntimeHealth};
use ce_core::ids::AudioStreamId;
use ce_core::ids::{CapabilityId, CommandId, CorrelationId, IdGenerator, SessionId};
use ce_core::intel::{AnalysisConfig, AnalysisStatus};
use ce_core::module::{ModuleDescriptor, ModuleFactory};
use ce_core::observability::{NoopSink, ObservabilitySink};
use ce_core::privacy::{ConsentPurpose, ConsentState};
use ce_core::resource::{AdmissionPolicy, QueueKind};
use ce_core::session::{SessionConfig, SessionInfo};
use ce_core::speech::{AsrCapabilities, RecognitionConfig, RecognitionStatus};
use ce_core::time::{Clock, SystemClock};
use ce_core::version::VersionReq;
pub use ce_speech as speech;
use ce_speech::{AsrFactory, LanguageIdentifier, WhichlangIdentifier};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use runtime::actor::Ingress;
use runtime::core::{Core, CoreDeps, CoreMsg, SpeechProviders};
use runtime::metrics::Metrics;
#[cfg(feature = "tracing")]
pub use tracing_sink::TracingSink;

/// Version of this facade's API. Provisional until CE declares 1.0.
pub const FACADE_API_VERSION: &str = "0.1";

/// Commonly used contract types.
pub mod prelude {
    pub use crate::{Engine, EngineBuilder, EventSubscription, SubscriptionError};
    pub use ce_audio::{AudioBackend, CaptureReader, PlaybackWriter, ReadError};
    pub use ce_core::audio::{
        AudioBlock, AudioDeviceId, AudioDirection, AudioEvent, AudioStreamState, AudioStreamStatus,
        ChannelMode, DeviceCatalog, DeviceSelector, FormatPreference, SampleFormat, StreamRequest,
    };
    pub use ce_core::capability::{CapabilityDescriptor, CapabilityInfo, CapabilityRequirement};
    pub use ce_core::command::{Command, CommandOutcome, ShutdownReport};
    pub use ce_core::config::{EngineConfig, ModuleConfig, RuntimeConfig};
    pub use ce_core::error::{CeError, ErrorCategory, Recoverability};
    pub use ce_core::event::{EventEnvelope, EventPayload, EventTopic, ModuleEventDraft};
    pub use ce_core::health::{HealthStatus, RuntimeHealth};
    pub use ce_core::ids::AudioStreamId;
    pub use ce_core::ids::{CapabilityId, EventName, ModuleId, SessionId};
    pub use ce_core::intel::{AnalysisConfig, AnalysisEvent, AnalysisStatus};
    pub use ce_core::lifecycle::{ModuleState, RuntimeState};
    pub use ce_core::module::{Module, ModuleContext, ModuleDescriptor, ModuleFactory};
    pub use ce_core::privacy::DataClass;
    pub use ce_core::privacy::{ConsentPurpose, ConsentState};
    pub use ce_core::session::{SessionConfig, SessionInfo, SessionState};
    pub use ce_core::speech::{
        LanguageObservation, RecognitionConfig, RecognitionStatus, RevisionStatus, SpeechEvent,
        TranscriptRevision,
    };
    pub use ce_core::version::{Version, VersionReq};
    pub use ce_core::{BoxFuture, CeResult};
}

/// Random (UUID v4) identifier generator — the production default.
#[derive(Debug, Default, Clone, Copy)]
pub struct RandomIdGenerator;

impl IdGenerator for RandomIdGenerator {
    fn generate(&self) -> uuid::Uuid {
        uuid::Uuid::new_v4()
    }
}

/// Configures and builds an [`Engine`].
#[derive(Debug, Default)]
pub struct EngineBuilder {
    config: EngineConfig,
    clock: Option<Arc<dyn Clock>>,
    ids: Option<Arc<dyn IdGenerator>>,
    sink: Option<Arc<dyn ObservabilitySink>>,
    audio: Option<Arc<dyn AudioBackend>>,
    speech: SpeechProviders,
    no_identifier: bool,
}

impl EngineBuilder {
    /// Builder with default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Use `config` (validated by [`EngineBuilder::build`]).
    pub fn config(mut self, config: EngineConfig) -> Self {
        self.config = config;
        self
    }

    /// Inject a clock (default: [`SystemClock`]).
    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = Some(clock);
        self
    }

    /// Inject an id generator (default: [`RandomIdGenerator`]).
    pub fn id_generator(mut self, ids: Arc<dyn IdGenerator>) -> Self {
        self.ids = Some(ids);
        self
    }

    /// Inject an observability sink (default: [`NoopSink`]).
    pub fn observability(mut self, sink: Arc<dyn ObservabilitySink>) -> Self {
        self.sink = Some(sink);
        self
    }

    /// Use an audio backend (Phase 2). Without one, audio operations fail
    /// with `audio.no_backend`.
    pub fn audio_backend(mut self, backend: Arc<dyn AudioBackend>) -> Self {
        self.audio = Some(backend);
        self
    }

    /// Register a speech-recognition adapter (Phase 4). The first registered
    /// adapter is the default; `RecognitionConfig::adapter` selects another
    /// by id. Registration is cheap: no model is loaded until a recognition
    /// starts, on that recognition's worker thread.
    pub fn asr_adapter(mut self, factory: impl AsrFactory + 'static) -> Self {
        self.speech.adapters.push(Arc::new(factory));
        self
    }

    /// Replace the text language identifier (default: whichlang, see
    /// ADR-0020).
    pub fn language_identifier(mut self, identifier: Arc<dyn LanguageIdentifier>) -> Self {
        self.speech.identifier = Some(identifier);
        self.no_identifier = false;
        self
    }

    /// Run without text language identification (observations then come
    /// only from a `Fixed` hint or an adapter that reports languages).
    pub fn without_language_identifier(mut self) -> Self {
        self.speech.identifier = None;
        self.no_identifier = true;
        self
    }

    /// Validate the configuration and spawn the runtime core.
    ///
    /// Must be called from within a Tokio runtime.
    pub fn build(self) -> CeResult<Engine> {
        self.config.validate()?;
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            CeError::new(
                ErrorCategory::Lifecycle,
                "runtime.no_async_runtime",
                "the engine must be built inside a Tokio runtime",
            )
        })?;
        let config = Arc::new(self.config);
        let rt = &config.runtime;
        let (cmd_tx, cmd_rx) = mpsc::channel(rt.command_queue_capacity.get());
        let (ingress_tx, ingress_rx) = mpsc::channel(rt.event_ingress_capacity.get());
        let (bus, _) = broadcast::channel(rt.event_broadcast_capacity.get());
        let metrics = Arc::new(Metrics::default());
        let root = CancellationToken::new();
        let ids = self.ids.unwrap_or_else(|| Arc::new(RandomIdGenerator));
        let engine_bus = bus.downgrade();
        let admission = rt.command_admission;
        let mut speech = self.speech;
        if speech.identifier.is_none() && !self.no_identifier {
            speech.identifier = Some(Arc::new(WhichlangIdentifier));
        }
        let core = Core::new(CoreDeps {
            config: config.clone(),
            clock: self.clock.unwrap_or_else(|| Arc::new(SystemClock::new())),
            ids: ids.clone(),
            sink: self.sink.unwrap_or_else(|| Arc::new(NoopSink)),
            metrics: metrics.clone(),
            root: root.clone(),
            bus,
            cmd_rx,
            ingress_tx: ingress_tx.clone(),
            ingress_rx,
            audio_backend: self.audio,
            speech,
        });
        let core = handle.spawn(core.run());
        Ok(Engine {
            cmd_tx,
            bus: engine_bus,
            ingress_tx,
            metrics,
            ids,
            admission,
            root,
            core: Some(core),
        })
    }
}

/// A running Communication Engine instance.
///
/// Dropping an `Engine` without calling [`Engine::shutdown`] triggers the same
/// bounded shutdown in the background (reason `HandleDropped`).
#[derive(Debug)]
pub struct Engine {
    cmd_tx: mpsc::Sender<CoreMsg>,
    bus: broadcast::WeakSender<Arc<EventEnvelope>>,
    ingress_tx: mpsc::Sender<Ingress>,
    metrics: Arc<Metrics>,
    ids: Arc<dyn IdGenerator>,
    admission: AdmissionPolicy,
    root: CancellationToken,
    core: Option<JoinHandle<()>>,
}

fn stopped() -> CeError {
    CeError::new(
        ErrorCategory::Shutdown,
        "runtime.stopped",
        "the runtime has shut down",
    )
}

impl Engine {
    /// A builder.
    pub fn builder() -> EngineBuilder {
        EngineBuilder::new()
    }

    /// Subscribe to every event dispatched from now on.
    ///
    /// Each subscription retains up to `event_broadcast_capacity` undelivered
    /// events; beyond that the oldest are overwritten and the next
    /// [`EventSubscription::recv`] reports how many were missed.
    pub fn subscribe(&self) -> CeResult<EventSubscription> {
        let sender = self.bus.upgrade().ok_or_else(stopped)?;
        Ok(EventSubscription {
            rx: sender.subscribe(),
            metrics: self.metrics.clone(),
            ingress: self.ingress_tx.clone(),
        })
    }

    async fn submit(&self, msg: CoreMsg) -> CeResult<()> {
        let rejected = |metrics: &Metrics| {
            Metrics::inc(&metrics.commands_rejected, 1);
            metrics.command_saturated.store(true, Ordering::Release);
            CeError::new(
                ErrorCategory::QueueSaturated,
                "queue.saturated",
                "command queue is full",
            )
            .with_context("queue", QueueKind::Command.to_string())
            .with_context("capacity", self.cmd_tx.max_capacity().to_string())
        };
        match self.admission.wait() {
            None => match self.cmd_tx.try_send(msg) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => return Err(rejected(&self.metrics)),
                Err(mpsc::error::TrySendError::Closed(_)) => return Err(stopped()),
            },
            Some(wait) => match tokio::time::timeout(wait, self.cmd_tx.reserve()).await {
                Ok(Ok(permit)) => permit.send(msg),
                Ok(Err(_)) => return Err(stopped()),
                Err(_) => return Err(rejected(&self.metrics)),
            },
        }
        Metrics::inc(&self.metrics.commands_accepted, 1);
        Ok(())
    }

    /// Execute a command with a fresh correlation id.
    pub async fn execute(&self, command: Command) -> CeResult<CommandOutcome> {
        let correlation = CorrelationId::generate(self.ids.as_ref());
        self.execute_correlated(command, correlation).await
    }

    /// Execute a command under a caller-supplied correlation id, which is
    /// carried into every event the command causes.
    pub async fn execute_correlated(
        &self,
        command: Command,
        correlation_id: CorrelationId,
    ) -> CeResult<CommandOutcome> {
        command.validate()?;
        let (reply, rx) = oneshot::channel();
        let envelope = CommandEnvelope {
            id: CommandId::generate(self.ids.as_ref()),
            correlation_id,
            command,
        };
        self.submit(CoreMsg::Command { envelope, reply }).await?;
        rx.await.map_err(|_| stopped())?
    }

    /// Register a module. Only its descriptor is read; it is instantiated
    /// when the runtime starts. Allowed only before [`Engine::start`].
    pub async fn register_module(&self, factory: impl ModuleFactory) -> CeResult<ModuleDescriptor> {
        let (reply, rx) = oneshot::channel();
        self.submit(CoreMsg::Register {
            factory: Arc::new(factory),
            reply,
        })
        .await?;
        rx.await.map_err(|_| stopped())?
    }

    /// Resolve dependencies, then initialize and start every module.
    /// Returns the number of running modules.
    pub async fn start(&self) -> CeResult<u32> {
        match self.execute(Command::Start).await? {
            CommandOutcome::Started { modules } => Ok(modules),
            other => Err(unexpected(&other)),
        }
    }

    /// Every registered capability.
    pub async fn capabilities(&self) -> CeResult<Vec<CapabilityInfo>> {
        self.list_capabilities(None, None).await
    }

    /// Providers of `capability` whose version matches `version`.
    pub async fn find_capability(
        &self,
        capability: &CapabilityId,
        version: &VersionReq,
    ) -> CeResult<Vec<CapabilityInfo>> {
        self.list_capabilities(Some(capability.clone()), Some(version.clone()))
            .await
    }

    async fn list_capabilities(
        &self,
        capability: Option<CapabilityId>,
        version: Option<VersionReq>,
    ) -> CeResult<Vec<CapabilityInfo>> {
        match self
            .execute(Command::ListCapabilities {
                capability,
                version,
            })
            .await?
        {
            CommandOutcome::Capabilities { capabilities } => Ok(capabilities),
            other => Err(unexpected(&other)),
        }
    }

    /// Open a session. Its correlation id is the command's correlation id.
    pub async fn create_session(&self, config: SessionConfig) -> CeResult<SessionInfo> {
        match self.execute(Command::CreateSession { config }).await? {
            CommandOutcome::SessionCreated { session } => Ok(session),
            other => Err(unexpected(&other)),
        }
    }

    /// Close a session.
    pub async fn close_session(&self, session_id: SessionId) -> CeResult<SessionInfo> {
        match self.execute(Command::CloseSession { session_id }).await? {
            CommandOutcome::SessionClosed { session } => Ok(session),
            other => Err(unexpected(&other)),
        }
    }

    /// Probe every module and report runtime health.
    pub async fn health(&self) -> CeResult<RuntimeHealth> {
        match self.execute(Command::QueryHealth).await? {
            CommandOutcome::Health { health } => Ok(health),
            other => Err(unexpected(&other)),
        }
    }

    // ------------------------------------------------------------ audio

    /// Re-enumerate audio devices and return the catalog. Device changes
    /// since the last enumeration are published as audio events.
    pub async fn audio_devices(&self) -> CeResult<DeviceCatalog> {
        match self.execute(Command::ListAudioDevices).await? {
            CommandOutcome::AudioDevices { catalog } => Ok(catalog),
            other => Err(unexpected(&other)),
        }
    }

    /// Record a consent decision for a session. Revoking capture consent
    /// stops capture immediately and releases the session's capture devices.
    pub async fn update_consent(
        &self,
        session_id: SessionId,
        purpose: ConsentPurpose,
        state: ConsentState,
    ) -> CeResult<SessionInfo> {
        match self
            .execute(Command::UpdateConsent {
                session_id,
                purpose,
                state,
            })
            .await?
        {
            CommandOutcome::ConsentUpdated { session } => Ok(session),
            other => Err(unexpected(&other)),
        }
    }

    async fn open_audio(
        &self,
        session_id: SessionId,
        request: StreamRequest,
    ) -> CeResult<StreamEndpoint> {
        let (reply, rx) = oneshot::channel();
        self.submit(CoreMsg::OpenAudio {
            session_id,
            request,
            reply,
        })
        .await?;
        rx.await.map_err(|_| stopped())?.map(|o| o.endpoint)
    }

    /// Open (not start) a capture stream in a session. Requires capture
    /// consent on the session and platform permission. The returned reader
    /// carries the stream's resolved format in `info()`.
    pub async fn open_capture(
        &self,
        session_id: SessionId,
        request: StreamRequest,
    ) -> CeResult<CaptureReader> {
        if request.direction != AudioDirection::Capture {
            return Err(CeError::invalid_input(
                "audio.direction_mismatch",
                "open_capture needs a capture request",
            ));
        }
        match self.open_audio(session_id, request).await? {
            StreamEndpoint::Capture(reader) => Ok(reader),
            StreamEndpoint::Playback(_) => Err(CeError::internal(
                "audio.endpoint_mismatch",
                "runtime returned a playback endpoint",
            )),
        }
    }

    /// Open (not start) a playback stream in a session.
    pub async fn open_playback(
        &self,
        session_id: SessionId,
        request: StreamRequest,
    ) -> CeResult<PlaybackWriter> {
        if request.direction != AudioDirection::Playback {
            return Err(CeError::invalid_input(
                "audio.direction_mismatch",
                "open_playback needs a playback request",
            ));
        }
        match self.open_audio(session_id, request).await? {
            StreamEndpoint::Playback(writer) => Ok(writer),
            StreamEndpoint::Capture(_) => Err(CeError::internal(
                "audio.endpoint_mismatch",
                "runtime returned a capture endpoint",
            )),
        }
    }

    async fn audio_command(&self, command: Command) -> CeResult<AudioStreamStatus> {
        match self.execute(command).await? {
            CommandOutcome::AudioStream { status } => Ok(status),
            other => Err(unexpected(&other)),
        }
    }

    /// Start (or restart) an open or stopped stream.
    pub async fn start_audio(&self, stream_id: AudioStreamId) -> CeResult<AudioStreamStatus> {
        self.audio_command(Command::StartAudioStream { stream_id })
            .await
    }

    /// Stop a running stream, keeping its device.
    pub async fn stop_audio(&self, stream_id: AudioStreamId) -> CeResult<AudioStreamStatus> {
        self.audio_command(Command::StopAudioStream { stream_id })
            .await
    }

    /// Close a stream and release its device; returns the final status.
    pub async fn close_audio(&self, stream_id: AudioStreamId) -> CeResult<AudioStreamStatus> {
        self.audio_command(Command::CloseAudioStream { stream_id })
            .await
    }

    /// Every audio stream that is not closed.
    pub async fn audio_streams(&self) -> CeResult<Vec<AudioStreamStatus>> {
        match self.execute(Command::ListAudioStreams).await? {
            CommandOutcome::AudioStreams { streams } => Ok(streams),
            other => Err(unexpected(&other)),
        }
    }

    async fn analysis_command(&self, command: Command) -> CeResult<AnalysisStatus> {
        match self.execute(command).await? {
            CommandOutcome::Analysis { status } => Ok(status),
            other => Err(unexpected(&other)),
        }
    }

    /// Start audio analysis (Phase 3) on a capture stream with the engine's
    /// configured detector. `config` overrides `EngineConfig::analysis`.
    /// Results arrive as `EventPayload::Analysis` events on the stream's
    /// session. Analysis never delays capture: when it falls behind, its
    /// own copy of the audio is dropped and reported as a discontinuity.
    pub async fn start_analysis(
        &self,
        stream_id: AudioStreamId,
        config: Option<AnalysisConfig>,
    ) -> CeResult<AnalysisStatus> {
        self.analysis_command(Command::StartAnalysis { stream_id, config })
            .await
    }

    /// Start analysis with a caller-supplied voice-activity detector (a
    /// replaceable adapter implementing [`audio_intel::VoiceActivityDetector`]).
    pub async fn start_analysis_with(
        &self,
        stream_id: AudioStreamId,
        config: Option<AnalysisConfig>,
        detector: Box<dyn audio_intel::VoiceActivityDetector>,
    ) -> CeResult<AnalysisStatus> {
        let (reply, rx) = oneshot::channel();
        self.submit(CoreMsg::StartAnalysisWith {
            stream_id,
            config,
            detector,
            reply,
        })
        .await?;
        match rx.await.map_err(|_| stopped())?? {
            CommandOutcome::Analysis { status } => Ok(status),
            other => Err(unexpected(&other)),
        }
    }

    /// Stop analysis on a stream; returns its final status.
    pub async fn stop_analysis(&self, stream_id: AudioStreamId) -> CeResult<AnalysisStatus> {
        self.analysis_command(Command::StopAnalysis { stream_id })
            .await
    }

    /// Current (or final) analysis status of a stream.
    pub async fn analysis_status(&self, stream_id: AudioStreamId) -> CeResult<AnalysisStatus> {
        self.analysis_command(Command::QueryAnalysis { stream_id })
            .await
    }

    async fn recognition_command(&self, command: Command) -> CeResult<RecognitionStatus> {
        match self.execute(command).await? {
            CommandOutcome::Recognition { status } => Ok(*status),
            other => Err(unexpected(&other)),
        }
    }

    /// Start speech recognition (Phase 4) on a capture stream with a
    /// registered adapter. `config` overrides `EngineConfig::speech`.
    /// Results arrive as `EventPayload::Speech` events on the stream's
    /// session: revisioned transcripts (`Content`) and language
    /// observations (`Personal`). With `ActivityHints` (the default) a
    /// Phase 3 analysis must be running on the stream. Recognition never
    /// delays capture: its own copy of the audio is dropped when it falls
    /// behind, and reported as a discontinuity.
    pub async fn start_recognition(
        &self,
        stream_id: AudioStreamId,
        config: Option<RecognitionConfig>,
    ) -> CeResult<RecognitionStatus> {
        self.recognition_command(Command::StartRecognition { stream_id, config })
            .await
    }

    /// Stop recognition; the open utterance is finalized. Returns the final
    /// status.
    pub async fn stop_recognition(&self, stream_id: AudioStreamId) -> CeResult<RecognitionStatus> {
        self.recognition_command(Command::StopRecognition { stream_id })
            .await
    }

    /// Cancel the open utterance and reset the recognizer; recognition
    /// continues with the next audio.
    pub async fn reset_recognition(&self, stream_id: AudioStreamId) -> CeResult<RecognitionStatus> {
        self.recognition_command(Command::ResetRecognition { stream_id })
            .await
    }

    /// Current (or final) recognition status.
    pub async fn recognition_status(
        &self,
        stream_id: AudioStreamId,
    ) -> CeResult<RecognitionStatus> {
        self.recognition_command(Command::QueryRecognition { stream_id })
            .await
    }

    /// Capabilities of the registered adapters (no model is loaded).
    pub async fn recognizers(&self) -> CeResult<Vec<AsrCapabilities>> {
        match self.execute(Command::ListRecognizers).await? {
            CommandOutcome::Recognizers { adapters } => Ok(adapters),
            other => Err(unexpected(&other)),
        }
    }

    /// Current counters (no round trip to the core).
    pub fn metrics(&self) -> MetricsSnapshot {
        self.metrics.snapshot()
    }

    /// Shut down: close sessions, stop modules in reverse dependency order
    /// within the configured timeouts, and wait for the core to exit.
    pub async fn shutdown(mut self) -> CeResult<ShutdownReport> {
        let outcome = self.execute(Command::Shutdown).await;
        if let Some(core) = self.core.take() {
            // The core exits right after replying; a join error here means it
            // panicked, which is a runtime bug worth surfacing.
            core.await.map_err(|e| {
                CeError::internal("runtime.core_panicked", "runtime core task failed")
                    .with_source(e)
            })?;
        }
        match outcome? {
            CommandOutcome::ShutdownCompleted { report } => Ok(report),
            other => Err(unexpected(&other)),
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if self.core.is_some() {
            self.root.cancel();
        }
    }
}

fn unexpected(outcome: &CommandOutcome) -> CeError {
    CeError::internal(
        "runtime.unexpected_outcome",
        "runtime returned an unexpected outcome",
    )
    .with_context("outcome", format!("{outcome:?}"))
}

/// Why [`EventSubscription::recv`] returned no event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionError {
    /// The subscriber fell behind and `missed` events were overwritten. The
    /// subscription remains usable; the next call returns the oldest retained
    /// event.
    Lagged {
        /// Number of events missed.
        missed: u64,
    },
    /// The runtime has shut down and every retained event has been received.
    Closed,
}

/// A stream of event envelopes in sequence order.
#[derive(Debug)]
pub struct EventSubscription {
    rx: broadcast::Receiver<Arc<EventEnvelope>>,
    metrics: Arc<Metrics>,
    ingress: mpsc::Sender<Ingress>,
}

impl EventSubscription {
    fn lagged(&self, missed: u64) -> SubscriptionError {
        Metrics::inc(&self.metrics.events_missed_by_subscribers, missed);
        // Best effort: make the loss visible as an event too. If the ingress
        // queue is full the counter above still records it.
        let _ = self.ingress.try_send(Ingress::SubscriberLagged { missed });
        SubscriptionError::Lagged { missed }
    }

    /// Next event.
    pub async fn recv(&mut self) -> Result<Arc<EventEnvelope>, SubscriptionError> {
        match self.rx.recv().await {
            Ok(event) => Ok(event),
            Err(broadcast::error::RecvError::Lagged(n)) => Err(self.lagged(n)),
            Err(broadcast::error::RecvError::Closed) => Err(SubscriptionError::Closed),
        }
    }

    /// Next event if one is ready.
    pub fn try_recv(&mut self) -> Result<Option<Arc<EventEnvelope>>, SubscriptionError> {
        match self.rx.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(broadcast::error::TryRecvError::Empty) => Ok(None),
            Err(broadcast::error::TryRecvError::Lagged(n)) => Err(self.lagged(n)),
            Err(broadcast::error::TryRecvError::Closed) => Err(SubscriptionError::Closed),
        }
    }
}
