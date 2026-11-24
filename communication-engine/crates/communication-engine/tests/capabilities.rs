//! Registry and capability discovery through the public facade.

mod common;

use ce_testkit::MockModuleFactory;
use ce_testkit::fixtures::{capability, module, requires};
use common::*;
use communication_engine::contracts::capability::{
    Cardinality, ExecutionLocation, LanguageSupport, LanguageTag,
};
use communication_engine::contracts::command::ModuleShutdownOutcome;
use communication_engine::contracts::event::StopReason;
use communication_engine::contracts::version::ContractVersion;
use communication_engine::prelude::*;

#[tokio::test]
async fn discovery_exposes_descriptors_without_instantiating_modules() {
    let mut h = harness();
    let cap = capability("ce.cap.asr", 2)
        .streaming(true)
        .execution(ExecutionLocation::OnDevice)
        .languages(LanguageSupport::Listed(vec![
            LanguageTag::new("hu").unwrap(),
        ]));
    let factory = MockModuleFactory::new(module("ce.test.asr").provides(cap.clone()));
    let probe = factory.probe();
    h.engine.register_module(factory).await.unwrap();

    let caps = h.engine.capabilities().await.unwrap();
    assert_eq!(caps.len(), 1);
    assert_eq!(caps[0].descriptor, cap);
    assert_eq!(caps[0].provider.as_str(), "ce.test.asr");
    assert_eq!(caps[0].provider_state, ModuleState::Discovered);
    // Unknown properties stay unknown.
    assert_eq!(caps[0].descriptor.inputs, None);
    assert_eq!(caps[0].descriptor.privacy.leaves_device, None);
    assert_eq!(probe.created(), 0, "discovery must not instantiate");

    h.engine.start().await.unwrap();
    assert_eq!(probe.created(), 1);
    let caps = h.engine.capabilities().await.unwrap();
    assert_eq!(caps[0].provider_state, ModuleState::Running);

    let registered = h
        .events
        .wait_for_kind(WAIT, "capability_registered")
        .await
        .unwrap();
    assert!(matches!(&registered.payload,
        EventPayload::CapabilityRegistered { capability, provider, .. }
            if capability.as_str() == "ce.cap.asr" && provider.as_str() == "ce.test.asr"));
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn find_capability_filters_by_version_requirement() {
    let h = harness();
    h.engine
        .register_module(MockModuleFactory::new(
            module("ce.test.v1").provides(capability("ce.cap.x", 1)),
        ))
        .await
        .unwrap();
    h.engine
        .register_module(MockModuleFactory::new(
            module("ce.test.v2").provides(capability("ce.cap.x", 2)),
        ))
        .await
        .unwrap();
    let id = CapabilityId::new("ce.cap.x").unwrap();
    let all = h
        .engine
        .find_capability(&id, &VersionReq::STAR)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    let v2 = h
        .engine
        .find_capability(&id, &VersionReq::parse("^2").unwrap())
        .await
        .unwrap();
    assert_eq!(v2.len(), 1);
    assert_eq!(v2[0].provider.as_str(), "ce.test.v2");
    let none = h
        .engine
        .find_capability(&id, &VersionReq::parse("^3").unwrap())
        .await
        .unwrap();
    assert!(none.is_empty());
    let other = CapabilityId::new("ce.cap.none").unwrap();
    assert!(
        h.engine
            .find_capability(&other, &VersionReq::STAR)
            .await
            .unwrap()
            .is_empty()
    );
    // A version filter without a capability filter is an invalid command.
    let err = h
        .engine
        .execute(Command::ListCapabilities {
            capability: None,
            version: Some(VersionReq::STAR),
        })
        .await
        .unwrap_err();
    assert_eq!(err.category(), ErrorCategory::InvalidInput);
    h.engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_registrations_are_rejected_and_the_runtime_stays_usable() {
    let h = harness();
    h.engine
        .register_module(MockModuleFactory::new(module("ce.test.a").provides(
            capability("ce.cap.only", 1).cardinality(Cardinality::Exclusive),
        )))
        .await
        .unwrap();
    let cases = [
        (module("ce.test.a"), "registry.duplicate_module"),
        (
            module("ce.test.b").provides(capability("ce.cap.only", 1)),
            "registry.exclusive_capability",
        ),
        (
            module("ce.test.c").conflicts_with(ModuleId::new("ce.test.a").unwrap()),
            "registry.module_conflict",
        ),
        (
            ModuleDescriptor {
                contract_version: ContractVersion { major: 9, minor: 0 },
                ..module("ce.test.d")
            },
            "registry.incompatible_contract",
        ),
        (
            module("ce.test.e")
                .provides(capability("ce.cap.e", 1))
                .provides(capability("ce.cap.e", 1)),
            "module.duplicate_capability",
        ),
        (
            ModuleDescriptor {
                name: String::new(),
                ..module("ce.test.f")
            },
            "module.invalid_name",
        ),
    ];
    for (descriptor, code) in cases {
        let err = h
            .engine
            .register_module(MockModuleFactory::new(descriptor))
            .await
            .unwrap_err();
        assert_eq!(
            err.category(),
            ErrorCategory::CapabilityResolution,
            "{code}"
        );
        assert_eq!(err.code(), code);
    }
    assert_eq!(h.engine.capabilities().await.unwrap().len(), 1);
    assert_eq!(h.engine.start().await.unwrap(), 1);
    h.engine.shutdown().await.unwrap();
}

async fn start_fails_with(
    dependent: ModuleDescriptor,
    providers: Vec<ModuleDescriptor>,
    code: &str,
) {
    let mut h = harness();
    let probe = {
        let f = MockModuleFactory::new(dependent);
        let p = f.probe();
        h.engine.register_module(f).await.unwrap();
        p
    };
    for p in providers {
        h.engine
            .register_module(MockModuleFactory::new(p))
            .await
            .unwrap();
    }
    let err = h.engine.start().await.unwrap_err();
    assert_eq!(
        (err.category(), err.code()),
        (ErrorCategory::CapabilityResolution, code)
    );
    assert_eq!(probe.created(), 0);
    let health = h.engine.health().await.unwrap();
    assert_eq!(health.state, RuntimeState::Failed);
    assert_eq!(health.modules[0].state, ModuleState::Failed);
    let report = h.engine.shutdown().await.unwrap();
    assert_eq!(report.reason, StopReason::StartFailed);
    assert!(!report.clean);
    assert_eq!(
        report
            .modules
            .iter()
            .find(|m| m.module.as_str() == "ce.test.dep")
            .unwrap()
            .outcome,
        ModuleShutdownOutcome::AlreadyFailed
    );
    h.events.drain_until_closed(WAIT).await;
    assert!(error_codes(&h.events).iter().any(|c| c == code));
    assert_eq!(
        module_transitions(&h.events, "ce.test.dep"),
        [
            ModuleState::Failed,
            ModuleState::Stopping,
            ModuleState::Stopped
        ]
    );
}

#[tokio::test]
async fn missing_dependencies_fail_start() {
    start_fails_with(
        module("ce.test.dep").requires(requires("ce.cap.missing", "*")),
        vec![],
        "registry.missing_dependency",
    )
    .await;
}

#[tokio::test]
async fn incompatible_versions_fail_start() {
    start_fails_with(
        module("ce.test.dep").requires(requires("ce.cap.base", "^2")),
        vec![module("ce.test.base").provides(capability("ce.cap.base", 1))],
        "registry.incompatible_version",
    )
    .await;
}

#[tokio::test]
async fn dependency_cycles_fail_start() {
    start_fails_with(
        module("ce.test.dep")
            .provides(capability("ce.cap.a", 1))
            .requires(requires("ce.cap.b", "*")),
        vec![
            module("ce.test.other")
                .provides(capability("ce.cap.b", 1))
                .requires(requires("ce.cap.a", "*")),
        ],
        "registry.dependency_cycle",
    )
    .await;
}
