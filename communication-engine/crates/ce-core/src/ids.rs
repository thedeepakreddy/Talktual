//! Typed identifiers.
//!
//! Two families exist:
//!
//! * **Opaque instance IDs** ([`EventId`], [`SessionId`], [`CorrelationId`],
//!   [`CommandId`]) wrap a UUID. They are generated through an injectable
//!   [`IdGenerator`] so tests can be deterministic.
//! * **Stable names** ([`ModuleId`], [`CapabilityId`], [`ProviderId`],
//!   [`EventName`], [`ModeId`]) are lowercase, dot-namespaced identifiers such as
//!   `ce.mock.echo`. They are part of contracts and must not change between
//!   releases of the thing they name.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{CeError, ErrorCategory};

/// Source of fresh UUIDs for instance identifiers.
///
/// The runtime ships a random (v4) generator; the test kit ships a sequential
/// one. Implementations must be cheap and must not block.
pub trait IdGenerator: Send + Sync + fmt::Debug {
    /// Return a new UUID. Values must be unique within one runtime.
    fn generate(&self) -> Uuid;
}

macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Wrap an existing UUID.
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// Generate a new identifier from `generator`.
            pub fn generate(generator: &dyn IdGenerator) -> Self {
                Self(generator.generate())
            }

            /// The underlying UUID.
            pub const fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "_{}"), self.0.simple())
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }
    };
}

uuid_id!(
    /// Identifies one event envelope.
    EventId,
    "evt"
);
uuid_id!(
    /// Identifies one session.
    SessionId,
    "ses"
);
uuid_id!(
    /// Ties together the command, events and errors that belong to one
    /// logical operation.
    CorrelationId,
    "cor"
);
uuid_id!(
    /// Identifies one submitted command.
    CommandId,
    "cmd"
);
uuid_id!(
    /// Identifies one audio stream (capture or playback). Added in Phase 2.
    AudioStreamId,
    "aud"
);

/// Maximum length of a namespaced identifier.
pub const MAX_NAME_LEN: usize = 128;

/// Validate a dot-namespaced name: at least two segments, each starting with a
/// lowercase ASCII letter and containing only `[a-z0-9_-]`.
fn validate_namespaced(kind: &'static str, value: &str) -> Result<(), CeError> {
    let invalid = |reason: &'static str| {
        CeError::new(
            ErrorCategory::InvalidInput,
            "id.invalid_name",
            format!("invalid {kind}: {reason}"),
        )
        .with_context("name", value.to_owned())
    };
    if value.is_empty() || value.len() > MAX_NAME_LEN {
        return Err(invalid("length must be 1..=128"));
    }
    let mut segments = 0;
    for segment in value.split('.') {
        segments += 1;
        let mut chars = segment.chars();
        match chars.next() {
            Some(c) if c.is_ascii_lowercase() => {}
            _ => return Err(invalid("each segment must start with a lowercase letter")),
        }
        if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
            return Err(invalid("segments may contain only a-z, 0-9, '_' and '-'"));
        }
    }
    if segments < 2 {
        return Err(invalid("must be namespaced, e.g. 'vendor.name'"));
    }
    Ok(())
}

macro_rules! name_id {
    ($(#[$meta:meta])* $name:ident, $kind:literal) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Parse and validate a name.
            pub fn new(value: impl Into<String>) -> Result<Self, CeError> {
                let value = value.into();
                validate_namespaced($kind, &value)?;
                Ok(Self(value))
            }

            /// The name as a string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = CeError;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = CeError;
            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> String {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({:?})", stringify!($name), self.0)
            }
        }
    };
}

name_id!(
    /// Stable identity of a module, e.g. `ce.mock.echo`.
    ModuleId,
    "module id"
);
name_id!(
    /// Stable identity of a capability, e.g. `ce.audio.capture`.
    CapabilityId,
    "capability id"
);
name_id!(
    /// Stable identity of an external provider adapter, e.g. `ce.mock.provider`.
    ProviderId,
    "provider id"
);
name_id!(
    /// Name of a module-defined event type, e.g. `ce.mock.session_ack`.
    EventName,
    "event name"
);
name_id!(
    /// Consumer-defined interaction mode attached to a session, e.g.
    /// `example.push_to_talk`. CE attaches no meaning to it.
    ModeId,
    "mode id"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaced_names_are_validated() {
        for ok in ["ce.mock", "ce.mock.echo", "a1.b_2.c-3", "vendor.x"] {
            assert!(ModuleId::new(ok).is_ok(), "{ok} should be valid");
        }
        for bad in [
            "",
            "single",
            "Upper.case",
            "ce..empty",
            "ce.9starts_digit",
            "ce.has space",
            "ce.dot.",
            ".ce.leading",
            "ce.unicode_é",
        ] {
            let err = ModuleId::new(bad).expect_err(bad);
            assert_eq!(err.category(), ErrorCategory::InvalidInput);
            assert_eq!(err.code(), "id.invalid_name");
        }
        let long = format!("ce.{}", "a".repeat(MAX_NAME_LEN));
        assert!(CapabilityId::new(long).is_err());
    }

    #[test]
    fn names_round_trip_through_serde_and_reject_invalid_input() {
        let id = CapabilityId::new("ce.audio.capture").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"ce.audio.capture\"");
        assert_eq!(serde_json::from_str::<CapabilityId>(&json).unwrap(), id);
        assert!(serde_json::from_str::<CapabilityId>("\"NotValid\"").is_err());
    }

    #[test]
    fn uuid_ids_display_with_type_prefix_and_round_trip() {
        let uuid = Uuid::from_u128(0x1234);
        let id = SessionId::from_uuid(uuid);
        assert_eq!(id.to_string(), format!("ses_{}", uuid.simple()));
        assert_eq!(format!("{id:?}"), id.to_string());
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<SessionId>(&json).unwrap(), id);
        assert_eq!(
            EventId::from_uuid(uuid).to_string().split('_').next(),
            Some("evt")
        );
    }

    #[derive(Debug)]
    struct Fixed;
    impl IdGenerator for Fixed {
        fn generate(&self) -> Uuid {
            Uuid::from_u128(7)
        }
    }

    #[test]
    fn ids_are_generated_through_the_injected_generator() {
        assert_eq!(
            CorrelationId::generate(&Fixed).as_uuid(),
            Uuid::from_u128(7)
        );
    }
}
