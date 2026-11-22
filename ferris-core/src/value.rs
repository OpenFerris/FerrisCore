use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::fmt;

use crate::Result;
use crate::error::ValidationError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Value>),
    Object(HashMap<String, Value>),
}

impl Value {
    pub fn null() -> Self {
        Value::Null
    }

    pub fn bool(v: bool) -> Self {
        Value::Bool(v)
    }

    pub fn number<N: Into<serde_json::Number>>(v: N) -> Self {
        Value::Number(v.into())
    }

    pub fn string<S: Into<String>>(v: S) -> Self {
        Value::String(v.into())
    }

    pub fn array(v: Vec<Value>) -> Self {
        Value::Array(v)
    }

    pub fn object(v: HashMap<String, Value>) -> Self {
        Value::Object(v)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Number(n) => n.as_i64(),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Number(n) => n.as_u64(),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => n.as_f64(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&HashMap<String, Value>> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object()?.get(key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.as_object_mut()?.get_mut(key)
    }

    fn as_object_mut(&mut self) -> Option<&mut HashMap<String, Value>> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn set(&mut self, key: String, value: Value) {
        if let Value::Object(map) = self {
            map.insert(key, value);
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.as_object_mut()?.remove(key)
    }

    pub fn to_json(&self) -> JsonValue {
        match self {
            Value::Null => JsonValue::Null,
            Value::Bool(b) => JsonValue::Bool(*b),
            Value::Number(n) => JsonValue::Number(n.clone()),
            Value::String(s) => JsonValue::String(s.clone()),
            Value::Array(a) => JsonValue::Array(a.iter().map(|v| v.to_json()).collect()),
            Value::Object(o) => {
                let mut map = serde_json::Map::new();
                for (k, v) in o {
                    map.insert(k.clone(), v.to_json());
                }
                JsonValue::Object(map)
            }
        }
    }

    pub fn from_json(v: JsonValue) -> Self {
        match v {
            JsonValue::Null => Value::Null,
            JsonValue::Bool(b) => Value::Bool(b),
            JsonValue::Number(n) => Value::Number(n),
            JsonValue::String(s) => Value::String(s),
            JsonValue::Array(a) => Value::Array(a.into_iter().map(Value::from_json).collect()),
            JsonValue::Object(o) => {
                let mut map = HashMap::new();
                for (k, v) in o {
                    map.insert(k, Value::from_json(v));
                }
                Value::Object(map)
            }
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_json())
    }
}

impl From<JsonValue> for Value {
    fn from(v: JsonValue) -> Self {
        Value::from_json(v)
    }
}

impl From<Value> for JsonValue {
    fn from(v: Value) -> Self {
        v.to_json()
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map(Into::into).unwrap_or(Value::Null)
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::Array(v.into_iter().map(Into::into).collect())
    }
}

impl<K: Into<String>, V: Into<Value>> From<HashMap<K, V>> for Value {
    fn from(v: HashMap<K, V>) -> Self {
        Value::Object(v.into_iter().map(|(k, v)| (k.into(), v.into())).collect())
    }
}

macro_rules! impl_from_primitive {
    ($($t:ty),*) => {
        $(impl From<$t> for Value {
            fn from(v: $t) -> Self {
                Value::Number(v.into())
            }
        })*
    };
}

impl_from_primitive!(i8, i16, i32, i64, u8, u16, u32, u64, isize, usize);

impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::from(v as f64)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        if v.is_finite() {
            // `from_f64` only returns `None` for NaN/±inf, which the guard above
            // already excluded — but a library path never unwraps, so an
            // impossible `None` still maps to `Null` rather than panicking.
            serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
        } else {
            // JSON has no NaN/±inf representation. Never silently coerce to 0
            // (data corruption): map to `Null`, which preserves "no value".
            Value::Null
        }
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

/// Build a [`Value`] from a float, **rejecting** non-finite inputs (`NaN`,
/// `±inf`) instead of silently coercing them. Use this whenever a `Null`
/// produced by [`From<f64>`] would mask an upstream bug.
///
/// # Errors
///
/// [`ValidationError::Invalid`] for a non-finite float: it is malformed input,
/// not an internal failure, and the caller can decide what to do with it.
pub fn from_f64_strict(v: f64) -> Result<Value> {
    if v.is_finite() {
        Ok(Value::from(v))
    } else {
        Err(
            ValidationError::Invalid(format!("non-finite float cannot be stored as a Value: {v}"))
                .into(),
        )
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_string())
    }
}

pub type Map = HashMap<String, Value>;

/// Macro for creating Value objects
#[macro_export]
macro_rules! value {
    (null) => { $crate::Value::null() };
    ({ $($key:tt : $val:tt),+ $(,)? }) => {{
        let mut map = std::collections::HashMap::new();
        $(map.insert($key.to_string(), $crate::value!($val));)+
        $crate::Value::Object(map)
    }};
    ([ $($val:tt),+ $(,)? ]) => {{
        $crate::Value::Array(vec![$($crate::value!($val)),+])
    }};
    ($val:expr) => { $crate::Value::from($val) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_creation() {
        let v = value!({
            "name": "test",
            "age": 30,
            "active": true,
            "tags": ["a", "b"],
            "nested": { "key": "value" }
        });

        assert_eq!(v.get("name").and_then(|v| v.as_str()), Some("test"));
        assert_eq!(v.get("age").and_then(|v| v.as_i64()), Some(30));
        assert_eq!(v.get("active").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            v.get("tags").and_then(|v| v.as_array()).map(|a| a.len()),
            Some(2)
        );
    }

    #[test]
    fn test_value_serialization() {
        let v = value!({ "key": "value" });
        let json = v.to_json();
        assert_eq!(json["key"], "value");
    }

    #[test]
    fn non_finite_floats_never_become_zero() {
        assert_eq!(Value::from(f64::NAN), Value::Null);
        assert_eq!(Value::from(f64::INFINITY), Value::Null);
        assert_eq!(Value::from(f64::NEG_INFINITY), Value::Null);
        assert_eq!(Value::from(f32::NAN), Value::Null);

        assert_eq!(Value::from(0.0_f64).as_f64(), Some(0.0));
        assert_eq!(
            Value::from(-42.5_f64).as_f64(),
            Some(-42.5),
            "finite floats must survive untouched"
        );
    }

    #[test]
    fn from_f64_strict_rejects_non_finite() {
        assert!(from_f64_strict(f64::NAN).is_err());
        assert!(from_f64_strict(f64::INFINITY).is_err());
        assert!(from_f64_strict(f64::NEG_INFINITY).is_err());
        assert_eq!(from_f64_strict(3.25).unwrap().as_f64(), Some(3.25));
    }
}
