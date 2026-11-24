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
