//! Device descriptors, the device catalog and platform permission.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::format::{StreamFormat, SupportedFormatRange};
use crate::error::ErrorReport;

/// CE's identifier for an audio device.
///
/// Backend identifiers are often unstable (names, indices) or contain
/// personal data (e.g. "Alex's headphones"). CE therefore maps each backend
/// key to an opaque, runtime-local id. The same backend key keeps the same id
/// for the lifetime of a runtime; ids are never reused for a different key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AudioDeviceId(pub u32);

impl fmt::Display for AudioDeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "dev_{}", self.0)
    }
}

/// How stable a backend's device key is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdStability {
    /// Stable across reconnects and restarts (e.g. a hardware UID).
    Stable,
    /// Stable only while the device stays connected / the process runs.
    Session,
    /// Not known.
    Unknown,
}

/// Description of one audio device.
///
/// `name` is classified **personal** (device names can identify people) and
/// is never logged or put in events by CE; events carry only `id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioDeviceDescriptor {
    /// CE device id.
    pub id: AudioDeviceId,
    /// Human-readable name (personal data).
    pub name: String,
    /// Stability of the backend key behind `id`.
    pub key_stability: IdStability,
    /// Whether capture streams can be opened.
    pub supports_input: bool,
    /// Whether playback streams can be opened.
    pub supports_output: bool,
    /// Whether this is the current default input.
    pub is_default_input: bool,
    /// Whether this is the current default output.
    pub is_default_output: bool,
    /// Supported capture formats (sanitized).
    pub input_formats: Vec<SupportedFormatRange>,
    /// Supported playback formats (sanitized).
    pub output_formats: Vec<SupportedFormatRange>,
    /// The device's preferred capture format, if reported.
    pub default_input_format: Option<StreamFormat>,
    /// The device's preferred playback format, if reported.
    pub default_output_format: Option<StreamFormat>,
    /// Formats skipped because CE cannot represent them or the backend
    /// reported them inconsistently.
    pub skipped_formats: u32,
}

/// Static facts about the audio backend in use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendInfo {
    /// Backend name, e.g. `cpal/alsa` or `fake`.
    pub name: String,
    /// Backend library version.
    pub version: String,
    /// Whether the backend pushes device-list change notifications. If not,
    /// CE detects changes by re-enumerating (on request or by polling).
    pub hotplug_notifications: bool,
    /// Whether the backend can query the platform's capture permission.
    pub permission_query: bool,
}

/// A snapshot of the devices a backend reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCatalog {
    /// Backend facts.
    pub backend: BackendInfo,
    /// Devices, in backend order.
    pub devices: Vec<AudioDeviceDescriptor>,
    /// Default input device, if any.
    pub default_input: Option<AudioDeviceId>,
    /// Default output device, if any.
    pub default_output: Option<AudioDeviceId>,
    /// Non-fatal problems met while enumerating (devices skipped, partial
    /// failures). Redacted.
    pub problems: Vec<ErrorReport>,
}

impl DeviceCatalog {
    /// Look up a device.
    pub fn device(&self, id: AudioDeviceId) -> Option<&AudioDeviceDescriptor> {
        self.devices.iter().find(|d| d.id == id)
    }

    /// Input-capable devices.
    pub fn inputs(&self) -> impl Iterator<Item = &AudioDeviceDescriptor> {
        self.devices.iter().filter(|d| d.supports_input)
    }

    /// Output-capable devices.
    pub fn outputs(&self) -> impl Iterator<Item = &AudioDeviceDescriptor> {
        self.devices.iter().filter(|d| d.supports_output)
    }
}

/// The platform's capture permission (e.g. a microphone privacy setting).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformPermission {
    /// The platform granted access.
    Granted,
    /// The user or policy denied access.
    Denied,
    /// The user has not been asked yet.
    NotDetermined,
    /// Access is restricted (e.g. parental or device-management policy).
    Restricted,
    /// The platform has no capture permission model for this backend (e.g.
    /// ALSA on desktop Linux, where access is governed by device-file
    /// permissions and failures surface at open time).
    NotRequiredByPlatform,
    /// The state cannot be determined. Treated as **not granted**.
    Unknown,
}

impl PlatformPermission {
    /// Whether capture may proceed as far as the platform is concerned.
    pub const fn allows_capture(self) -> bool {
        matches!(
            self,
            PlatformPermission::Granted | PlatformPermission::NotRequiredByPlatform
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_granted_or_not_required_permissions_allow_capture() {
        use PlatformPermission::*;
        for (p, ok) in [
            (Granted, true),
            (NotRequiredByPlatform, true),
            (Denied, false),
            (NotDetermined, false),
            (Restricted, false),
            (Unknown, false),
        ] {
            assert_eq!(p.allows_capture(), ok, "{p:?}");
        }
        assert_eq!(AudioDeviceId(3).to_string(), "dev_3");
    }
}
