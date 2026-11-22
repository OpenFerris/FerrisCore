//! The canonical fieldtype vocabulary: one list, one normalizer.
//!
//! Ferris ships three families of `fieldtype` producers — the `#[field]`
//! attribute macro, the `FieldMeta` builders and WASM plugins' `describe()` —
//! plus rows persisted by the customization endpoint. All of them use this one
//! list.
//!
//! [`canonical`] is the single place that answers "which spelling does the wire
//! use?": Frappe's, verbatim from
//! <https://docs.frappe.io/framework/user/en/basics/doctypes/fieldtypes>, plus
//! the Ferris extras documented on [`FIELDTYPES`]. It returns `None` for
//! anything else.
//!
//! Coverage of that page is pinned by `every_documented_frappe_fieldtype_is_known`:
//! all 38 documented fieldtypes are in [`FIELDTYPES`], so a name can only leave
//! the list together with the test that fails. The companion page
//! <https://docs.frappe.io/framework/user/en/basics/doctypes/docfield> defines
//! the *properties* a field carries (`label`, `fieldname`, `fieldtype`, `reqd`,
//! `options`, `default`, `depends_on`, …); this module owns only the
//! `fieldtype` half of that, which is why [`FIELDTYPES`] and
//! [`crate::registry::FieldMeta`] are separate things.
//!
//! Where an unknown fieldtype must be **rejected** (rather than normalized):
//! the customization write path, the WASM descriptor registration and the
//! stored-doctype load path all call [`canonical`] and refuse `None` — that is
//! the "allowlist única estricta" the user asked for. `SchemaRegistry::register`
//! stays infallible (its callers include macro-generated `OnceLock`
//! initializers, where an error cannot surface without a panic) and
//! canonicalizes as a last line of defense.
#![allow(clippy::upper_case_acronyms)] // `HTML`/`JSON`/`URL` are wire values.

/// Every canonical fieldtype name, in the wire spelling.
///
/// The Frappe baseline plus the Ferris extras: `Week`, `File`, `Heading` (a
/// caption-only form element), `Widget` (a plugin/desk widget rendered in place,
/// whose `options` names `<app>/<widget>` — see the Desk's `WidgetField`;
/// valueless, so it never reaches a document payload), `Secret` (a recoverable
/// credential — see `ferris_core::secret`; Frappe only has the one-way
/// `Password`), and the typed string validators `Phone`, `Email`, `URL` and
/// `Color`, which Frappe models as `Data` with an `Options` validation instead
/// of as their own types.
pub const FIELDTYPES: &[&str] = &[
    "Data",
    "Link",
    "Dynamic Link",
    "Select",
    "Table",
    "Table MultiSelect",
    "Check",
    "Int",
    "Float",
    "Currency",
    "Percent",
    "Rating",
    "Duration",
    "Date",
    "Datetime",
    "Time",
    "Week",
    "Text",
    "Small Text",
    "Long Text",
    "Text Editor",
    "Markdown Editor",
    "Code",
    "HTML",
    "HTML Editor",
    "JSON",
    "Password",
    "Secret",
    "Color",
    "Image",
    "File",
    "Attach",
    "Attach Image",
    "Attachment Gallery",
    "Signature",
    "Barcode",
    "Geolocation",
    "Phone",
    "Email",
    "URL",
    "Read Only",
    "Button",
    "Heading",
    "Widget",
    "Tab Break",
    "Section Break",
    "Column Break",
];

/// Resolve `name` to its canonical wire spelling, or `None` when it is not a
/// known fieldtype.
///
/// Matching is case-sensitive, as Frappe is. Surrounding whitespace is ignored,
/// since this also runs on user input from the "Personalizar" dialog.
#[must_use]
pub fn canonical(name: &str) -> Option<&'static str> {
    let trimmed = name.trim();
    // The canonical spelling *is* the list entry: consulting `FIELDTYPES`
    // instead of matching every name one by one is what keeps the two from
    // drifting — a name added to the list needs no second edit here.
    if let Some(known) = FIELDTYPES.iter().copied().find(|&f| f == trimmed) {
        return Some(known);
    }
    None
}

/// Whether `name` is a fieldtype the framework knows.
#[must_use]
pub fn is_known(name: &str) -> bool {
    canonical(name).is_some()
}

/// The structural breaks: fieldtypes that only arrange the form and never
/// carry a value.
#[must_use]
pub fn is_layout(name: &str) -> bool {
    matches!(
        canonical(name).unwrap_or(name),
        "Tab Break" | "Section Break" | "Column Break"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list and the normalizer must not drift: every canonical name
    /// resolves to itself, and nothing else does.
    #[test]
    fn canonical_names_resolve_to_themselves() {
        assert!(!FIELDTYPES.is_empty());
        for name in FIELDTYPES {
            assert_eq!(canonical(name), Some(*name), "canonical {name}");
        }
    }

    /// Frappe's documented fieldtypes, transcribed one-to-one from
    /// <https://docs.frappe.io/framework/user/en/basics/doctypes/fieldtypes>
    /// (updated 2026-07-16). Two headings are spelled differently from the
    /// wire: the docs' "Date and Time" is `Datetime` and "Int (Integer)" is
    /// `Int`.
    ///
    /// This is the coverage check the vocabulary exists for: if a documented
    /// type ever leaves [`FIELDTYPES`], this test names it instead of the
    /// failure surfacing much later as a field that degrades to plain text.
    const DOCUMENTED_FRAPPE_FIELDTYPES: [&str; 38] = [
        "Data",
        "Link",
        "Dynamic Link",
        "Check",
        "Select",
        "Table",
        "Attach",
        "Attachment Gallery",
        "Attach Image",
        "Text Editor",
        "Date",
        "Datetime",
        "Barcode",
        "Button",
        "Code",
        "Color",
        "Column Break",
        "Currency",
        "Float",
        "Geolocation",
        "HTML",
        "Image",
        "Int",
        "Small Text",
        "Long Text",
        "Text",
        "Markdown Editor",
        "Password",
        "Percent",
        "Rating",
        "Read Only",
        "Section Break",
        "Tab Break",
        "Signature",
        "Table MultiSelect",
        "Time",
        "Duration",
        "JSON",
    ];

    #[test]
    fn every_documented_frappe_fieldtype_is_known() {
        assert_eq!(DOCUMENTED_FRAPPE_FIELDTYPES.len(), 38);
        for name in DOCUMENTED_FRAPPE_FIELDTYPES {
            assert!(is_known(name), "{name} is documented but unknown here");
            assert!(
                FIELDTYPES.contains(&name),
                "{name} is missing from the canonical list"
            );
        }
    }

    #[test]
    fn unknown_names_are_rejected() {
        for bad in ["", "Dat", "Long Int", "Icon", "datetime", "small text"] {
            assert_eq!(canonical(bad), None, "unknown {bad:?}");
        }
        // Trimming is deliberate: the Personalizar dialog sends raw input.
        assert_eq!(canonical(" Data"), Some("Data"));
        assert_eq!(canonical("Data "), Some("Data"));
    }

    /// `Secret` is a Ferris extra, not a Frappe fieldtype: it exists so a
    /// credential that must be *recoverable* (OAuth client secret, webhook
    /// signing key) is not declared `Password` and silently read back as a mask.
    /// It is in the canonical list, so every strict allowlist accepts it.
    #[test]
    fn secret_is_a_known_ferris_extra() {
        assert_eq!(canonical("Secret"), Some("Secret"));
        assert!(is_known("Secret"));
        assert!(
            !DOCUMENTED_FRAPPE_FIELDTYPES.contains(&"Secret"),
            "Secret is not part of Frappe's documented set"
        );
    }

    /// `Widget` is the second Ferris extra: a field that hosts a widget, named
    /// by its `options` (`<app>/<widget>`). It carries no value, which is why
    /// the Desk's validation/payload skip it like `Button`; the strict
    /// allowlists only have to accept the name.
    #[test]
    fn widget_is_a_known_ferris_extra_with_no_value_of_its_own() {
        assert_eq!(canonical("Widget"), Some("Widget"));
        assert!(is_known("Widget"));
        assert!(!is_layout("Widget"));
        assert!(
            !DOCUMENTED_FRAPPE_FIELDTYPES.contains(&"Widget"),
            "Widget is not part of Frappe's documented set"
        );
    }

    #[test]
    fn layout_breaks_are_recognized() {
        for name in ["Tab Break", "Section Break", "Column Break"] {
            assert!(is_layout(name), "layout {name}");
        }
        for name in ["Data", "Table", "Heading", "Button", ""] {
            assert!(!is_layout(name), "not layout {name}");
        }
    }
}
