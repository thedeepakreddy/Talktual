//! A small structured value type used for module configuration and
//! module-defined event attributes.
//!
//! CE deliberately does not expose `serde_json::Value` (or any other format's
//! value type) in its contracts, so hosts can use any serde format and the core
//! carries no format dependency.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A reference to a secret held outside CE configuration (an environment
/// variable, OS keychain entry or secret manager key). CE never stores the
/// secret value itself; resolving the reference is an adapter concern.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretRef {
    /// Name of the secret in the host's secret store.
    pub secret_ref: String,
}

/// Structured, format-neutral value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    /// Absent value.
    Null,
    /// Boolean.
    Bool(bool),
    /// Signed integer.
    Integer(i64),
    /// Floating point number.
    Float(f64),
    /// UTF-8 string. Never use for secrets; see [`Value::Secret`].
    String(String),
    /// Reference to an externally held secret.
    Secret(SecretRef),
    /// Ordered list.
    List(Vec<Value>),
    /// String-keyed map with deterministic ordering.
    Map(BTreeMap<String, Value>),
}

impl Value {
    /// The value as a string, if it is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// The value as an integer, if it is one.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    /// The value as a boolean, if it is one.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::String(value.to_owned())
    }
}
impl From<String> for Value {
    fn from(value: String) -> Self {
        Value::String(value)
    }
}
impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Value::Integer(value)
    }
}
impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Bool(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untagged_serde_distinguishes_secret_refs_from_maps() {
        let secret: Value = serde_json::from_str(r#"{"secret_ref":"PROVIDER_KEY"}"#).unwrap();
        assert_eq!(
            secret,
            Value::Secret(SecretRef {
                secret_ref: "PROVIDER_KEY".into()
            })
        );
        let map: Value = serde_json::from_str(r#"{"secret_ref":"x","other":1}"#).unwrap();
        assert!(matches!(map, Value::Map(_)));
        let list: Value = serde_json::from_str(r#"[1, 2.5, "a", true, null]"#).unwrap();
        assert_eq!(
            list,
            Value::List(vec![
                Value::Integer(1),
                Value::Float(2.5),
                Value::String("a".into()),
                Value::Bool(true),
                Value::Null
            ])
        );
    }
}
