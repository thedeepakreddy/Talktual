//! Module registry: registration-time validation and start-time resolution.

use std::collections::VecDeque;
use std::sync::Arc;

use ce_core::capability::{CapabilityInfo, Cardinality};
use ce_core::error::{CeError, ErrorCategory};
use ce_core::ids::{CapabilityId, ModuleId};
use ce_core::lifecycle::ModuleState;
use ce_core::module::{ModuleDescriptor, ModuleFactory};
use ce_core::version::{ContractVersion, VersionReq};

pub(crate) struct Registration {
    pub(crate) factory: Arc<dyn ModuleFactory>,
    pub(crate) descriptor: ModuleDescriptor,
}

impl std::fmt::Debug for Registration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registration")
            .field("module", &self.descriptor.id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
pub(crate) struct Registry {
    modules: Vec<Registration>,
}

fn resolution(code: &'static str, message: &'static str) -> CeError {
    CeError::new(ErrorCategory::CapabilityResolution, code, message)
}

impl Registry {
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.modules.len()
    }

    pub(crate) fn get(&self, index: usize) -> &Registration {
        &self.modules[index]
    }

    pub(crate) fn contains(&self, id: &ModuleId) -> bool {
        self.modules.iter().any(|m| &m.descriptor.id == id)
    }

    /// Validate and add a module. Only the descriptor is read; nothing is
    /// instantiated.
    pub(crate) fn register(
        &mut self,
        factory: Arc<dyn ModuleFactory>,
        runtime_contract: ContractVersion,
    ) -> Result<usize, CeError> {
        let descriptor = factory.descriptor();
        descriptor.validate()?;
        let ctx = |e: CeError| e.with_context("module", descriptor.id.to_string());
        if !descriptor
            .contract_version
            .is_supported_by(runtime_contract)
        {
            return Err(ctx(resolution(
                "registry.incompatible_contract",
                "module was built against an unsupported contract version",
            )
            .with_context("module_contract", descriptor.contract_version.to_string())
            .with_context("runtime_contract", runtime_contract.to_string())));
        }
        for existing in &self.modules {
            let other = &existing.descriptor;
            if other.id == descriptor.id {
                return Err(ctx(resolution(
                    "registry.duplicate_module",
                    "a module with this id is already registered",
                )));
            }
            if other.conflicts_with.contains(&descriptor.id)
                || descriptor.conflicts_with.contains(&other.id)
            {
                return Err(ctx(resolution(
                    "registry.module_conflict",
                    "module conflicts with a registered module",
                )
                .with_context("conflicts_with", other.id.to_string())));
            }
            for cap in &descriptor.provides {
                if let Some(theirs) = other.provides.iter().find(|c| c.id == cap.id) {
                    if cap.cardinality == Cardinality::Exclusive
                        || theirs.cardinality == Cardinality::Exclusive
                    {
                        return Err(ctx(resolution(
                            "registry.exclusive_capability",
                            "an exclusive capability already has a provider",
                        )
                        .with_context("capability", cap.id.to_string())
                        .with_context("provider", other.id.to_string())));
                    }
                }
            }
        }
        self.modules.push(Registration {
            factory,
            descriptor,
        });
        Ok(self.modules.len() - 1)
    }

    /// Resolve dependencies and return a start order in which every provider
    /// precedes its dependents. Ties keep registration order. On failure,
    /// returns every module that cannot be resolved with its error.
    pub(crate) fn resolve(&self) -> Result<Vec<usize>, Vec<(usize, CeError)>> {
        let n = self.modules.len();
        let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut indegree = vec![0usize; n];
        let mut failures = Vec::new();

        for (i, module) in self.modules.iter().enumerate() {
            for req in &module.descriptor.requires {
                let mut offered = false;
                let mut matched = Vec::new();
                for (j, other) in self.modules.iter().enumerate() {
                    if j == i {
                        continue;
                    }
                    if let Some(cap) = other
                        .descriptor
                        .provides
                        .iter()
                        .find(|c| c.id == req.capability)
                    {
                        offered = true;
                        if req.version.matches(&cap.version) {
                            matched.push(j);
                        }
                    }
                }
                if matched.is_empty() && !req.optional {
                    let err = if offered {
                        resolution(
                            "registry.incompatible_version",
                            "no provider offers a compatible version of a required capability",
                        )
                    } else {
                        resolution(
                            "registry.missing_dependency",
                            "no registered module provides a required capability",
                        )
                    };
                    failures.push((
                        i,
                        err.with_context("module", module.descriptor.id.to_string())
                            .with_context("capability", req.capability.to_string())
                            .with_context("required", req.version.to_string()),
                    ));
                }
                for j in matched {
                    if !dependents[j].contains(&i) {
                        dependents[j].push(i);
                        indegree[i] += 1;
                    }
                }
            }
        }
        if !failures.is_empty() {
            return Err(failures);
        }

        // Kahn's algorithm, always taking the lowest ready index for stability.
        let mut ready: VecDeque<usize> = (0..n).filter(|&i| indegree[i] == 0).collect();
        let mut order = Vec::with_capacity(n);
        while let Some(i) = ready.pop_front() {
            order.push(i);
            for &d in &dependents[i] {
                indegree[d] -= 1;
                if indegree[d] == 0 {
                    let pos = ready.iter().position(|&r| r > d).unwrap_or(ready.len());
                    ready.insert(pos, d);
                }
            }
        }
        if order.len() < n {
            return Err((0..n)
                .filter(|i| !order.contains(i))
                .map(|i| {
                    (
                        i,
                        resolution(
                            "registry.dependency_cycle",
                            "module is part of a dependency cycle",
                        )
                        .with_context("module", self.modules[i].descriptor.id.to_string()),
                    )
                })
                .collect());
        }
        Ok(order)
    }

    /// Registered capabilities, optionally filtered by id and version.
    pub(crate) fn capabilities(
        &self,
        capability: Option<&CapabilityId>,
        version: Option<&VersionReq>,
        state_of: impl Fn(usize) -> ModuleState,
    ) -> Vec<CapabilityInfo> {
        let mut out = Vec::new();
        for (i, module) in self.modules.iter().enumerate() {
            for cap in &module.descriptor.provides {
                if capability.is_some_and(|c| c != &cap.id) {
                    continue;
                }
                if version.is_some_and(|v| !v.matches(&cap.version)) {
                    continue;
                }
                out.push(CapabilityInfo {
                    descriptor: cap.clone(),
                    provider: module.descriptor.id.clone(),
                    provider_state: state_of(i),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ce_core::capability::{CapabilityDescriptor, CapabilityRequirement};
    use ce_core::config::ModuleConfig;
    use ce_core::module::Module;
    use ce_core::version::Version;

    struct F(ModuleDescriptor);
    impl ModuleFactory for F {
        fn descriptor(&self) -> ModuleDescriptor {
            self.0.clone()
        }
        fn create(&self, _: &ModuleConfig) -> Result<Box<dyn Module>, CeError> {
            Err(CeError::internal("test.no_instance", "not used"))
        }
    }

    fn module(id: &str) -> ModuleDescriptor {
        ModuleDescriptor::new(ModuleId::new(id).unwrap(), id, Version::new(1, 0, 0))
    }
    fn cap(id: &str, major: u64) -> CapabilityDescriptor {
        CapabilityDescriptor::new(CapabilityId::new(id).unwrap(), Version::new(major, 0, 0))
    }
    fn req(id: &str, v: &str) -> CapabilityRequirement {
        CapabilityRequirement::required(
            CapabilityId::new(id).unwrap(),
            VersionReq::parse(v).unwrap(),
        )
    }
    fn reg(r: &mut Registry, d: ModuleDescriptor) -> Result<usize, CeError> {
        r.register(Arc::new(F(d)), ContractVersion::CURRENT)
    }

    #[test]
    fn duplicates_conflicts_exclusive_capabilities_and_contracts_are_rejected_at_registration() {
        let mut r = Registry::default();
        reg(
            &mut r,
            module("ce.t.a").provides(cap("ce.cap.x", 1).cardinality(Cardinality::Exclusive)),
        )
        .unwrap();
        assert_eq!(
            reg(&mut r, module("ce.t.a")).unwrap_err().code(),
            "registry.duplicate_module"
        );
        assert_eq!(
            reg(&mut r, module("ce.t.b").provides(cap("ce.cap.x", 1)))
                .unwrap_err()
                .code(),
            "registry.exclusive_capability"
        );
        assert_eq!(
            reg(
                &mut r,
                module("ce.t.c").conflicts_with(ModuleId::new("ce.t.a").unwrap())
            )
            .unwrap_err()
            .code(),
            "registry.module_conflict"
        );
        let mut future = module("ce.t.d");
        future.contract_version = ContractVersion { major: 2, minor: 0 };
        assert_eq!(
            reg(&mut r, future).unwrap_err().code(),
            "registry.incompatible_contract"
        );
        // Shared capabilities may have several providers.
        reg(&mut r, module("ce.t.e").provides(cap("ce.cap.y", 1))).unwrap();
        reg(&mut r, module("ce.t.f").provides(cap("ce.cap.y", 1))).unwrap();
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn resolution_orders_providers_before_dependents() {
        let mut r = Registry::default();
        reg(&mut r, module("ce.t.app").requires(req("ce.cap.mid", "^1"))).unwrap();
        reg(
            &mut r,
            module("ce.t.mid")
                .provides(cap("ce.cap.mid", 1))
                .requires(req("ce.cap.base", "^2")),
        )
        .unwrap();
        reg(&mut r, module("ce.t.base").provides(cap("ce.cap.base", 2))).unwrap();
        reg(&mut r, module("ce.t.free")).unwrap();
        assert_eq!(r.resolve().unwrap(), vec![2, 1, 0, 3]);
    }

    #[test]
    fn missing_incompatible_optional_and_cyclic_dependencies() {
        let mut r = Registry::default();
        reg(&mut r, module("ce.t.a").requires(req("ce.cap.none", "*"))).unwrap();
        reg(&mut r, module("ce.t.b").requires(req("ce.cap.v", "^2"))).unwrap();
        reg(&mut r, module("ce.t.c").provides(cap("ce.cap.v", 1))).unwrap();
        let errs = r.resolve().unwrap_err();
        let codes: Vec<_> = errs
            .iter()
            .map(|(i, e)| (*i, e.code().to_owned()))
            .collect();
        assert_eq!(
            codes,
            vec![
                (0, "registry.missing_dependency".to_owned()),
                (1, "registry.incompatible_version".to_owned())
            ]
        );

        let mut r = Registry::default();
        reg(
            &mut r,
            module("ce.t.opt").requires(CapabilityRequirement::optional(
                CapabilityId::new("ce.cap.none").unwrap(),
                VersionReq::STAR,
            )),
        )
        .unwrap();
        assert_eq!(r.resolve().unwrap(), vec![0]);

        let mut r = Registry::default();
        reg(
            &mut r,
            module("ce.t.x")
                .provides(cap("ce.cap.x", 1))
                .requires(req("ce.cap.y", "*")),
        )
        .unwrap();
        reg(
            &mut r,
            module("ce.t.y")
                .provides(cap("ce.cap.y", 1))
                .requires(req("ce.cap.x", "*")),
        )
        .unwrap();
        reg(&mut r, module("ce.t.z")).unwrap();
        let errs = r.resolve().unwrap_err();
        assert_eq!(errs.len(), 2);
        assert!(
            errs.iter()
                .all(|(_, e)| e.code() == "registry.dependency_cycle")
        );
    }

    #[test]
    fn capability_listing_filters_by_id_and_version() {
        let mut r = Registry::default();
        reg(
            &mut r,
            module("ce.t.a")
                .provides(cap("ce.cap.x", 1))
                .provides(cap("ce.cap.y", 1)),
        )
        .unwrap();
        reg(&mut r, module("ce.t.b").provides(cap("ce.cap.x", 2))).unwrap();
        let all = r.capabilities(None, None, |_| ModuleState::Discovered);
        assert_eq!(all.len(), 3);
        let x = CapabilityId::new("ce.cap.x").unwrap();
        assert_eq!(
            r.capabilities(Some(&x), None, |_| ModuleState::Discovered)
                .len(),
            2
        );
        let v2 = VersionReq::parse("^2").unwrap();
        let only = r.capabilities(Some(&x), Some(&v2), |_| ModuleState::Running);
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].provider.as_str(), "ce.t.b");
        assert_eq!(only[0].provider_state, ModuleState::Running);
    }
}
