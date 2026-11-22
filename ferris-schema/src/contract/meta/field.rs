//! Field-level mirror of the registry metadata.

use serde::{Deserialize, Serialize};

/// Subset of `FieldMeta` the Desk actually renders.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DynField {
    pub fieldname: String,
    pub label: Option<String>,
    pub fieldtype: String,
    pub options: Option<String>,
    pub reqd: bool,
    pub read_only: bool,
    pub hidden: bool,
    pub in_list_view: bool,
    pub in_standard_filter: bool,
    /// Reject duplicate values across documents. The customization engine
    /// applies a `unique` setter, so the form editor has to be able to read the
    /// effective value back; additive and defaulted for older payloads.
    #[serde(default)]
    pub unique: bool,
    /// Frappe's **Virtual** DocField
    /// (<https://docs.frappe.io/framework/user/en/basics/virtual_docfield>):
    /// the field is not persisted, its value is computed on read, and the form
    /// renders it read-only. Additive and defaulted so older payloads parse.
    ///
    /// Written as a raw identifier because `virtual` is a reserved Rust keyword;
    /// the explicit `rename` keeps it under the plain name `virtual` on the wire.
    #[serde(rename = "virtual", default)]
    pub r#virtual: bool,
    /// Data Masking: the value is returned masked unless the caller holds the
    /// `mask` permission at this field's `permlevel`.
    #[serde(default)]
    pub mask: bool,
    /// Frappe's **Allow on Submit**
    /// (<https://docs.frappe.io/framework/doctypes/allow-on-submit>): even on a
    /// submitted document the Desk keeps this one field editable, which is how
    /// a status/remark field may be updated after submission. Defaulted for
    /// payloads emitted before it travelled on the wire.
    #[serde(default)]
    pub allow_on_submit: bool,
    /// Layout / validation / display hints (all safe to default).
    #[serde(default)]
    pub columns: u8,
    #[serde(default)]
    pub permlevel: u8,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub depends_on: Option<String>,
    #[serde(default)]
    pub min_value: Option<String>,
    #[serde(default)]
    pub max_value: Option<String>,
    #[serde(default)]
    pub precision: Option<u8>,
    #[serde(default)]
    pub max_length: Option<u32>,
    #[serde(default)]
    pub length: Option<u32>,
    #[serde(default)]
    pub read_only_on_load: bool,
    #[serde(default)]
    pub default: Option<String>,
    /// Render the label in bold, as Frappe's `bold` field property does.
    #[serde(default)]
    pub bold: bool,
    /// `"source_field.target_field"`: when `source_field` (a `Link`) changes,
    /// copy `target_field` of the linked document into this field.
    #[serde(default)]
    pub fetch_from: Option<String>,
    /// Only apply [`Self::fetch_from`] when this field is currently empty.
    #[serde(default)]
    pub fetch_if_empty: bool,
    #[serde(default)]
    pub non_negative: bool,
    #[serde(default)]
    pub collapsible: bool,
    #[serde(default)]
    pub collapsible_depends_on: Option<String>,
    #[serde(default)]
    pub mandatory_depends_on: Option<String>,
    #[serde(default)]
    pub read_only_depends_on: Option<String>,
    #[serde(default)]
    pub hide_border: bool,
    #[serde(default)]
    pub in_filter: bool,
    #[serde(default)]
    pub remember_last_selected_value: bool,
    #[serde(default)]
    pub ignore_xss_filter_field: bool,
    // The remaining `FieldMeta` properties. They were engine-only until now, so
    // a Desk or a plugin could not see the effective value of a customization
    // that set them. Defaulted, like the rest: an older payload keeps parsing.
    #[serde(default)]
    pub in_global_search: bool,
    #[serde(default)]
    pub in_preview: bool,
    #[serde(default)]
    pub print_hide: bool,
    #[serde(default)]
    pub report_hide: bool,
    #[serde(default)]
    pub search_index: bool,
    #[serde(default)]
    pub sort_options: bool,
    #[serde(default)]
    pub no_copy: bool,
    #[serde(default)]
    pub ignore_user_permissions: bool,
    #[serde(default)]
    pub ignore_xss_filter: bool,
    #[serde(default)]
    pub convert_to_user_timezone: bool,
    #[serde(default)]
    pub print_width: Option<String>,
    #[serde(default)]
    pub width: Option<String>,
}

/// The fields an index covers, and whether it is unique.
///
/// Only the fields are mirrored from the registry's `IndexMeta`: the physical
/// index name and type are database details the Desk has no use for.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DynIndex {
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub unique: bool,
}
