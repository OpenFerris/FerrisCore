use std::collections::{HashMap, HashSet};

use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json;
use validator::Validate;

/// Trait alias for types that can be used with forms
pub trait FormData:
    Validate + Clone + Default + Serialize + for<'de> Deserialize<'de> + 'static
{
}

/// Blanket implementation for all types that satisfy the bounds
impl<T> FormData for T where
    T: Validate + Clone + Default + Serialize + for<'de> Deserialize<'de> + 'static
{
}

/// Type alias for form field value setter function
pub type SetValueFn = Box<dyn Fn(&str, String) + Send + Sync>;

/// Type alias for form field touch function (called on blur)
pub type TouchFieldFn = Box<dyn Fn(&str) + Send + Sync>;

pub struct Form<T> {
    pub values_signal: RwSignal<HashMap<String, String>>,
    pub errors_signal: RwSignal<HashMap<String, Option<String>>>,
    pub touched_signal: RwSignal<HashSet<String>>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T> Clone for Form<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Form<T> {}

impl<T> Default for Form<T> {
    fn default() -> Self {
        Self {
            values_signal: RwSignal::new(Default::default()),
            errors_signal: RwSignal::new(Default::default()),
            touched_signal: RwSignal::new(Default::default()),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T> Form<T>
where
    T: FormData,
{
    pub fn value(&self, field: &str) -> String {
        self.values_signal
            .get()
            .get(field)
            .cloned()
            .unwrap_or_default()
    }

    pub fn error(&self, field: &str) -> Option<String> {
        self.errors_signal.get().get(field).and_then(Clone::clone)
    }

    /// Updates the field value without triggering validation.
    /// Validation happens on blur via `touch_field`.
    pub fn set_value(&self, field: &str, value: String) {
        let field = field.to_string();

        self.values_signal.update(|values| {
            values.insert(field.clone(), value);
        });

        // If field was already touched, re-validate on each change
        if self.is_touched(&field) {
            let error = self.validate_field(&field);
            self.errors_signal.update(|errors| {
                errors.insert(field, error);
            });
        }
    }

    /// Marks a field as touched (called on blur) and validates it.
    /// Errors are only shown for touched fields.
    pub fn touch_field(&self, field: &str) {
        let field = field.to_string();

        self.touched_signal.update(|touched| {
            touched.insert(field.clone());
        });

        // Validate on blur
        let error = self.validate_field(&field);
        self.errors_signal.update(|errors| {
            errors.insert(field, error);
        });
    }

    /// Check if a field has been touched (blurred at least once)
    pub fn is_touched(&self, field: &str) -> bool {
        self.touched_signal.get().contains(field)
    }

    fn validate_field(&self, field: &str) -> Option<String> {
        let data = self.map_to_struct(&self.values_signal.get())?;

        data.validate()
            .err()?
            .field_errors()
            .get(field)?
            .first()?
            .message
            .as_ref()
            .map(|m| m.to_string())
    }

    fn map_to_struct(&self, values: &HashMap<String, String>) -> Option<T> {
        map_values_to_struct(values)
    }

    /// Whether every field that already has a recorded error is clean.
    ///
    /// This is **not** "the form is valid": a freshly opened form has no
    /// recorded errors and returns `true` with its required fields empty. Gate
    /// submission on [`Self::can_submit`] instead.
    pub fn has_no_field_errors(&self) -> bool {
        self.errors_signal.get().values().all(Option::is_none)
    }

    pub fn can_submit(&self) -> bool {
        let Some(data) = self.map_to_struct(&self.values_signal.get()) else {
            return false;
        };
        data.validate().is_ok()
    }

    pub fn reset(&self) {
        self.values_signal.set(Default::default());
        self.errors_signal.set(Default::default());
        self.touched_signal.set(Default::default());
    }

    pub fn get_data(&self) -> Option<T> {
        self.map_to_struct(&self.values_signal.get())
    }

    pub fn validate_and_get(&self) -> Result<T, String> {
        let data = self
            .map_to_struct(&self.values_signal.get())
            .ok_or_else(|| "Please fill in all required fields.".to_string())?;

        data.validate().map_err(|errors| {
            errors
                .field_errors()
                .values()
                .filter_map(|errs| errs.first())
                .filter_map(|err| err.message.as_ref())
                .map(|msg| msg.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        })?;

        Ok(data)
    }
}

pub fn use_form<T>() -> Form<T>
where
    T: FormData,
{
    Form::default()
}

#[derive(Clone, Copy)]
pub struct FormContext {
    pub values_signal: RwSignal<HashMap<String, String>>,
    pub errors_signal: RwSignal<HashMap<String, Option<String>>>,
    pub touched_signal: RwSignal<HashSet<String>>,
    pub set_value: StoredValue<SetValueFn>,
    pub touch_field: StoredValue<TouchFieldFn>,
}

#[derive(Clone)]
pub struct FieldContext {
    pub name: String,
}

/// Trait for structs that can automatically generate form fields.
///
/// This trait is automatically implemented by the `#[derive(AutoForm)]` macro.
pub trait AutoFormFields: FormData {
    /// Renders all form fields for this struct.
    fn render_fields(form: Form<Self>) -> impl IntoView;
}

/// Build `T` from the form's string values.
///
/// Coercion is **type-directed**: a value is turned into a number/bool only when
/// the struct's own default for that field already has that JSON type, so a
/// `String` field whose value merely looks numeric (a phone number, a postal
/// code, a SKU) stays a string. Deciding from the value's shape alone made such
/// a form impossible to deserialize, and the failure surfaced as a disabled
/// submit button with no field error (UI4).
///
/// The attempts are ordered from least to most coercion:
/// 1. every value as the string the form stores;
/// 2. coerce the fields whose default is a number/bool (the common typed form);
/// 3. also coerce fields whose default is `null` (an `Option<number>`/`Option<bool>`
///    field), which only runs when the first two could not deserialize.
///
/// Free function rather than a method so the coercion can be tested without a
/// reactive runtime.
fn map_values_to_struct<T>(values: &HashMap<String, String>) -> Option<T>
where
    T: FormData,
{
    let default_value = serde_json::to_value(T::default()).ok()?;
    let default_map: HashMap<String, serde_json::Value> =
        serde_json::from_value(default_value).ok()?;

    let as_object = |map: HashMap<String, serde_json::Value>| {
        serde_json::Value::Object(map.into_iter().collect())
    };

    let as_strings: HashMap<String, serde_json::Value> = default_map
        .iter()
        .map(|(key, default)| {
            let value = values.get(key).filter(|value| !value.is_empty());
            let json = match value {
                Some(value) => serde_json::Value::String(value.clone()),
                None => default.clone(),
            };
            (key.clone(), json)
        })
        .collect();

    if let Ok(typed) = serde_json::from_value(as_object(as_strings.clone())) {
        return Some(typed);
    }

    // Fields the struct declares as numeric/boolean: the default says so.
    let mut typed_fields = as_strings.clone();
    for (key, default) in &default_map {
        let Some(value) = values.get(key).filter(|value| !value.is_empty()) else {
            continue;
        };
        if let Some(json) = coerce_to(value, default) {
            typed_fields.insert(key.clone(), json);
        }
    }
    if let Ok(typed) = serde_json::from_value(as_object(typed_fields.clone())) {
        return Some(typed);
    }

    // `Option<number>`/`Option<bool>` fields default to `null`; try those too.
    for (key, default) in &default_map {
        let Some(value) = values.get(key).filter(|value| !value.is_empty()) else {
            continue;
        };
        if default.is_null()
            && let Some(json) = coerce_to(value, &serde_json::Value::Bool(false))
        {
            typed_fields.insert(key.clone(), json);
        }
    }
    serde_json::from_value(as_object(typed_fields)).ok()
}

/// `value` as the JSON type of `default`, or `None` when it does not apply.
fn coerce_to(value: &str, default: &serde_json::Value) -> Option<serde_json::Value> {
    match default {
        serde_json::Value::Number(_) => {
            if let Ok(num) = value.parse::<i64>() {
                Some(serde_json::Value::Number(num.into()))
            } else {
                value
                    .parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map(serde_json::Value::Number)
            }
        }
        serde_json::Value::Bool(_) => match value {
            "true" => Some(serde_json::Value::Bool(true)),
            "false" => Some(serde_json::Value::Bool(false)),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Default, Serialize, Deserialize, Validate)]
    struct Sample {
        code: String,
        amount: i64,
        note: String,
    }

    /// UI4: a `String` field whose value is all digits must stay a string, and a
    /// numeric field must still receive a number.
    #[test]
    fn numeric_looking_strings_stay_strings() {
        let values = HashMap::from([
            ("code".to_string(), "12345".to_string()),
            ("amount".to_string(), "42".to_string()),
        ]);
        let sample: Sample = map_values_to_struct(&values).expect("both fields deserialize");
        assert_eq!(sample.code, "12345");
        assert_eq!(sample.amount, 42);
    }

    #[test]
    fn empty_values_keep_the_struct_default() {
        let values = HashMap::from([("note".to_string(), String::new())]);
        let sample: Sample = map_values_to_struct(&values).expect("defaults apply");
        assert_eq!(sample.note, "");
    }

    #[test]
    fn an_uncoercible_value_reports_failure() {
        let values = HashMap::from([("amount".to_string(), "not-a-number".to_string())]);
        assert!(map_values_to_struct::<Sample>(&values).is_none());
    }
}
