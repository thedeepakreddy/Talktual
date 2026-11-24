//! Version types and compatibility rules.
//!
//! * Module and capability versions use Semantic Versioning ([`Version`],
//!   [`VersionReq`] re-exported from the `semver` crate).
//! * The module contract — the shape of the [`crate::module`] traits — has its
//!   own [`ContractVersion`]. A module built against contract `M.m` runs on a
//!   runtime implementing contract `M.n` iff the majors match and `m <= n`.

use std::fmt;

use serde::{Deserialize, Serialize};

pub use semver::{Version, VersionReq};

/// Version of the module contract (major breaking, minor additive).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContractVersion {
    /// Incremented for breaking changes.
    pub major: u16,
    /// Incremented for backwards-compatible additions.
    pub minor: u16,
}

impl ContractVersion {
    /// The contract version this crate defines.
    pub const CURRENT: ContractVersion = ContractVersion { major: 1, minor: 0 };

    /// Whether a module built against `self` can run on a runtime that
    /// implements `runtime`.
    pub const fn is_supported_by(self, runtime: ContractVersion) -> bool {
        self.major == runtime.major && self.minor <= runtime.minor
    }
}

impl fmt::Display for ContractVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_compatibility_requires_same_major_and_not_newer_minor() {
        let rt = ContractVersion { major: 1, minor: 2 };
        assert!(ContractVersion { major: 1, minor: 0 }.is_supported_by(rt));
        assert!(ContractVersion { major: 1, minor: 2 }.is_supported_by(rt));
        assert!(!ContractVersion { major: 1, minor: 3 }.is_supported_by(rt));
        assert!(!ContractVersion { major: 2, minor: 0 }.is_supported_by(rt));
        assert!(!ContractVersion { major: 0, minor: 9 }.is_supported_by(rt));
        assert_eq!(ContractVersion::CURRENT.to_string(), "1.0");
    }
}
