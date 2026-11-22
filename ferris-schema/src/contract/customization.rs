//! Runtime metadata customization, as it crosses the wire.
//!
//! These are the payloads of `GET/POST/DELETE /api/customization/{doctype}`:
//! the Desk's "Customize Form" surface reads the current custom fields and
//! property setters, adds or replaces one, and removes one — all persisted in
//! `tab_custom_field` / `tab_property_setter` and applied to a `DocType`'s
//! effective metadata on the next request.
//!
//! They mirror `ferris_schema::customization::{CustomField, PropertySetter}`
//! (which need a database and so live behind the `native` feature) in the
//! pure-serde contract module the browser compiles, for the same reason
//! [`super::meta`] mirrors the registry: a server-side field added to the store
//! must not silently widen what the Desk can send.

use serde::{Deserialize, Serialize};

/// A field added to a `DocType` at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomFieldSpec {
    pub fieldname: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub fieldtype: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<String>,
    /// Insert the field after this fieldname (else append at the end).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insert_after: Option<String>,
    #[serde(default)]
    pub reqd: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub read_only: bool,
}

/// An override of one `DocType` or field property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropertySetterSpec {
    /// `None` targets a `DocType`-level property.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fieldname: Option<String>,
    pub property: String,
    pub value: String,
}

/// Everything customized about one `DocType`: the `GET` payload and the shape
/// the Customize Form editor renders.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Customization {
    #[serde(default)]
    pub fields: Vec<CustomFieldSpec>,
    #[serde(default)]
    pub properties: Vec<PropertySetterSpec>,
}

/// Which persisted changes an export has to carry.
///
/// The Desk lets an editor export *what the user changed in this session*
/// instead of the whole `DocType`, so a world file stays a reviewable diff
/// rather than a snapshot of every customization someone else made earlier.
/// The enum is defined here — not in `ferris-api` — because the browser
/// serializes it and the server deserializes it; one definition is what keeps
/// the two spellings from drifting.
///
/// Variant names are part of the wire format on purpose: the client sends
/// `{"Fields": ["priority"]}` or
/// `{"Properties": [[null, "title_field"]]}`, matching the deriving type
/// exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportScope {
    /// Export only these custom fields; property setters are dropped.
    Fields(Vec<String>),
    /// Export only these `(fieldname, property)` setters (`None` is a
    /// `DocType`-level setter); custom fields are dropped.
    Properties(Vec<(Option<String>, String)>),
}

/// The on-disk payload of a customization inside a plugin world
/// (`plugins/<app>/customizations/<doctype>.json`).
///
/// The same data as [`Customization`] plus the `DocType` it belongs to. The
/// file name is a sanitized stem (`Task-Line` for `Task Line`), so the name
/// cannot be recovered from the path: carrying it here is what lets a
/// deployment read the world back and apply it on `ferris migrate`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldCustomization {
    pub doctype: String,
    #[serde(flatten)]
    pub customization: Customization,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `POST` body only needs to name the field and its type; every other
    /// attribute has a default, so the Desk does not have to send them.
    #[test]
    fn custom_field_spec_defaults_are_optional() {
        let spec: CustomFieldSpec = serde_json::from_value(serde_json::json!({
            "fieldname": "priority",
            "fieldtype": "Select"
        }))
        .expect("spec parses");
        assert_eq!(spec.fieldname, "priority");
        assert_eq!(spec.fieldtype, "Select");
        assert!(spec.label.is_none());
        assert!(!spec.reqd && !spec.hidden && !spec.read_only);
    }

    /// A `DocType`-level property setter carries no `fieldname`; the round trip
    /// must keep it absent rather than turning it into an empty string, which
    /// would target a field named "" instead.
    #[test]
    fn doctype_level_property_setter_round_trips_without_fieldname() {
        let spec = PropertySetterSpec {
            fieldname: None,
            property: "title_field".to_string(),
            value: "title".to_string(),
        };
        let json = serde_json::to_value(&spec).expect("spec serializes");
        assert!(json.get("fieldname").is_none());
        let back: PropertySetterSpec = serde_json::from_value(json).expect("spec parses");
        assert_eq!(back, spec);
    }

    /// The scope is serialized by the Desk and deserialized by the API, so its
    /// spelling is part of the wire format: a rename would make every export
    /// silently fall back to "write everything".
    #[test]
    fn export_scope_wire_shape_is_stable() {
        let fields_wire = serde_json::json!({ "Fields": ["priority"] });
        let fields: ExportScope = serde_json::from_value(fields_wire).expect("fields scope parses");
        assert_eq!(fields, ExportScope::Fields(vec!["priority".to_string()]));

        let wire = serde_json::json!({
            "Properties": [[null, "title_field"], ["priority", "placeholder"]]
        });
        let properties: ExportScope =
            serde_json::from_value(wire.clone()).expect("properties scope parses");
        assert_eq!(
            properties,
            ExportScope::Properties(vec![
                (None, "title_field".to_string()),
                (Some("priority".to_string()), "placeholder".to_string()),
            ])
        );

        // And the round trip keeps the spelling the server expects back.
        let json = serde_json::to_value(&properties).expect("scope serializes");
        assert_eq!(json, wire);
    }

    /// An empty customization is a valid response: a `DocType` nobody has changed.
    #[test]
    fn empty_customization_parses() {
        let custom: Customization =
            serde_json::from_value(serde_json::json!({})).expect("customization parses");
        assert!(custom.fields.is_empty());
        assert!(custom.properties.is_empty());
    }
}
