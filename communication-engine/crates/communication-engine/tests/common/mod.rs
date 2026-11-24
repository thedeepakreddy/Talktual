//! Shared helpers for integration tests. Only public APIs are used.
#![allow(dead_code, unreachable_pub)]

use std::sync::Arc;
use std::time::Duration;

use ce_testkit::{EventRecorder, ManualClock, RecordingSink, SequentialIds, fixtures};
use communication_engine::contracts::config::EngineConfig;
use communication_engine::prelude::*;

/// Generous wall-clock bound for waiting on events in tests.
pub const WAIT: Duration = Duration::from_secs(5);

pub struct Harness {
    pub engine: Engine,
    pub events: EventRecorder,
    pub clock: Arc<ManualClock>,
    pub sink: Arc<RecordingSink>,
}

pub fn harness() -> Harness {
    harness_with(fixtures::test_config())
}

pub fn harness_with(config: EngineConfig) -> Harness {
    let clock = Arc::new(ManualClock::default());
    let sink = Arc::new(RecordingSink::default());
    let engine = Engine::builder()
        .config(config)
        .clock(clock.clone())
        .id_generator(Arc::new(SequentialIds::default()))
        .observability(sink.clone())
        .build()
        .expect("engine builds");
    let events = EventRecorder::new(engine.subscribe().expect("subscribe"));
    Harness {
        engine,
        events,
        clock,
        sink,
    }
}

type Hook = Box<dyn Fn(&ModuleContext) -> Result<(), CeError> + Send + Sync>;

/// A module driven by closures, for edge cases the mock does not script.
pub struct ScriptFactory {
    pub descriptor: ModuleDescriptor,
    pub on_start: Arc<Hook>,
    pub on_stop: Arc<Hook>,
}

impl ScriptFactory {
    pub fn new(descriptor: ModuleDescriptor) -> Self {
        Self {
            descriptor,
            on_start: Arc::new(Box::new(|_| Ok(()))),
            on_stop: Arc::new(Box::new(|_| Ok(()))),
        }
    }

    pub fn on_start(
        mut self,
        f: impl Fn(&ModuleContext) -> Result<(), CeError> + Send + Sync + 'static,
    ) -> Self {
        self.on_start = Arc::new(Box::new(f));
        self
    }

    pub fn on_stop(
        mut self,
        f: impl Fn(&ModuleContext) -> Result<(), CeError> + Send + Sync + 'static,
    ) -> Self {
        self.on_stop = Arc::new(Box::new(f));
        self
    }
}

struct ScriptModule {
    on_start: Arc<Hook>,
    on_stop: Arc<Hook>,
}

impl ModuleFactory for ScriptFactory {
    fn descriptor(&self) -> ModuleDescriptor {
        self.descriptor.clone()
    }
    fn create(&self, _: &ModuleConfig) -> Result<Box<dyn Module>, CeError> {
        Ok(Box::new(ScriptModule {
            on_start: self.on_start.clone(),
            on_stop: self.on_stop.clone(),
        }))
    }
}

impl Module for ScriptModule {
    fn initialize<'a>(&'a mut self, _: &'a ModuleContext) -> BoxFuture<'a, Result<(), CeError>> {
        Box::pin(async { Ok(()) })
    }
    fn start<'a>(&'a mut self, ctx: &'a ModuleContext) -> BoxFuture<'a, Result<(), CeError>> {
        let r = (self.on_start)(ctx);
        Box::pin(async move { r })
    }
    fn stop<'a>(&'a mut self, ctx: &'a ModuleContext) -> BoxFuture<'a, Result<(), CeError>> {
        let r = (self.on_stop)(ctx);
        Box::pin(async move { r })
    }
}

/// Module state transitions recorded for `module`, in order.
pub fn module_transitions(events: &EventRecorder, module: &str) -> Vec<ModuleState> {
    events
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::ModuleStateChanged { module: m, to, .. } if m.as_str() == module => {
                Some(*to)
            }
            _ => None,
        })
        .collect()
}

pub fn error_codes(events: &EventRecorder) -> Vec<String> {
    events
        .events()
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::RuntimeError { error, .. } => Some(error.code.clone()),
            _ => None,
        })
        .collect()
}

// ------------------------------------------------------------------ audio

use ce_testkit::fake_audio::{FakeAudioBackend, FakeDriver};
use communication_engine::contracts::audio::AudioEvent;
use communication_engine::contracts::time::WallTime;

pub struct AudioHarness {
    pub engine: Engine,
    pub events: EventRecorder,
    pub fake: FakeAudioBackend,
    pub clock: Arc<ManualClock>,
    pub session: SessionInfo,
}

/// Audio test configuration: fast housekeeping and pressure reports.
pub fn audio_config() -> EngineConfig {
    let mut config = fixtures::test_config();
    config.audio.housekeeping_interval_ms = 50;
    config.audio.pressure_report_interval_ms = 10;
    config.audio.operation_timeout_ms = 500;
    config.audio.open_timeout_ms = 1_000;
    config
}

/// A started engine with a fake backend and one session (no consent yet).
pub async fn audio_harness_with(config: EngineConfig, driver: FakeDriver) -> AudioHarness {
    audio_harness_custom(config, driver, |b| b).await
}

/// Like [`audio_harness_with`], with extra builder configuration (e.g.
/// speech adapters).
pub async fn audio_harness_custom(
    config: EngineConfig,
    driver: FakeDriver,
    customize: impl FnOnce(EngineBuilder) -> EngineBuilder,
) -> AudioHarness {
    let fake = FakeAudioBackend::new(driver);
    let clock = Arc::new(ManualClock::default());
    let engine = customize(
        Engine::builder()
            .config(config)
            .clock(clock.clone())
            .id_generator(Arc::new(SequentialIds::default()))
            .audio_backend(Arc::new(fake.clone())),
    )
    .build()
    .expect("engine builds");
    let events = EventRecorder::new(engine.subscribe().expect("subscribe"));
    engine.start().await.expect("start");
    let session = engine
        .create_session(SessionConfig::default())
        .await
        .expect("session");
    AudioHarness {
        engine,
        events,
        fake,
        clock,
        session,
    }
}

pub async fn audio_harness() -> AudioHarness {
    audio_harness_with(audio_config(), FakeDriver::Manual).await
}

impl AudioHarness {
    pub fn now(&self) -> WallTime {
        use communication_engine::contracts::time::Clock;
        self.clock.wall()
    }

    pub async fn grant_capture(&self) {
        self.engine
            .update_consent(
                self.session.id,
                ConsentPurpose::Capture,
                ConsentState::Granted { at: self.now() },
            )
            .await
            .expect("consent");
    }

    /// Audio events recorded so far.
    pub fn audio_events(&self) -> Vec<AudioEvent> {
        self.events
            .events()
            .iter()
            .filter_map(|e| match &e.payload {
                EventPayload::Audio(a) => Some(a.clone()),
                _ => None,
            })
            .collect()
    }

    pub async fn wait_audio(&mut self, pred: impl Fn(&AudioEvent) -> bool) -> AudioEvent {
        let e = self
            .events
            .wait_for(
                WAIT,
                |e| matches!(&e.payload, EventPayload::Audio(a) if pred(a)),
            )
            .await
            .expect("expected audio event");
        match &e.payload {
            EventPayload::Audio(a) => a.clone(),
            _ => unreachable!(),
        }
    }
}

/// Receive one block, failing the test after `WAIT`.
pub async fn recv_block(reader: &mut CaptureReader) -> AudioBlock {
    tokio::time::timeout(WAIT, reader.recv())
        .await
        .expect("block in time")
        .expect("block")
}
