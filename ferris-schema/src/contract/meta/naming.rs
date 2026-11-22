use serde::{Deserialize, Serialize};

/// How the engine mints the `name` (primary key) of a new document.
///
/// Frappe lets a doctype *declare* its naming strategy rather than relying on
/// the implicit "series if one exists, otherwise a hash". This enum mirrors
/// the nine methods Frappe documents
/// (<https://docs.frappe.io/framework/user/en/basics/doctypes/naming>), plus
/// Ferris' static `series` pattern:
///
/// | Ferris | Frappe's "Naming" option |
/// |---|---|
/// | [`NamingType::Series`] | *(Ferris)* the static metadata pattern |
/// | [`NamingType::NamingSeries`] | **By Naming Series field** — formats the document's `naming_series` value |
/// | [`NamingType::AutoIncrement`] | **Autoincrement** |
/// | [`NamingType::Field`] | **By fieldname** |
/// | [`NamingType::Uuid`] | **UUID** |
/// | [`NamingType::Prompt`] | **Set by user** |
/// | [`NamingType::Expression`] | **Expression** (dot-separated, e.g. `EXAMPLE-.MM.-test-.{fieldname1}.-.#####`) |
/// | [`NamingType::ExpressionOldStyle`] | **Expression (old style)**, e.g. `EXAMPLE-{MM}-{fieldname}-{#####}` |
/// | [`NamingType::Random`] | **Random** |
/// | [`NamingType::Script`] | **Script** — a registered Rust naming provider instead of a Python script |
///
/// This enum is that declaration, shared by the registry (which owns the
/// metadata) and the Desk (which has to know what, if anything, the user may
/// type into the id field). The value is resolved once, through
/// [`NamingType::resolve`], so the engine and the Desk cannot disagree about
/// what an undeclared doctype does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamingType {
    /// A naming series with a counter and optional date parts (`TASK-.#####`),
    /// stored in `tab_series`. This is Ferris' historical declaration and what
    /// an empty declaration with a non-empty `naming_series` resolves to.
    ///
    /// It is *not* Frappe's "By Naming Series field": here the pattern lives in
    /// the doctype metadata, not in a document field.
    Series,
    /// Frappe's **By Naming Series field**: read the new document's
    /// `naming_series` field and format that pattern, so an operator can choose
    /// the series per document instead of the doctype fixing one.
    NamingSeries,
    /// An opaque UUID v4. What an empty declaration with no series resolves to;
    /// the user never types it.
    Uuid,
    /// A bare, zero-padded-unless-the-series-says-otherwise numeric counter
    /// (`1`, `2`, …), with its own counter independent of any series.
    AutoIncrement,
    /// The name is the value of the field named by `naming_field`.
    Field,
    /// The caller must supply `name`; the engine refuses to mint one.
    Prompt,
    /// Frappe's dot-separated **Expression** language: each `.` separates a
    /// literal, date part, field reference or counter
    /// (`EXAMPLE-.MM.-test-.{fieldname1}.-.#####`).
    Expression,
    /// Frappe's **Expression (old style)**: one braces-delimited template
    /// (`EXAMPLE-{MM}-{fieldname}-{#####}`).
    ExpressionOldStyle,
    /// A random, non-sequential name. Unlike [`NamingType::Uuid`] the shape is
    /// an implementation detail, but it is never derived from the data.
    Random,
    /// Frappe's **Script**, adapted: naming runs in a registered Rust provider
    /// rather than a Python script, so a plugin can mint names without the host
    /// embedding a scripting engine.
    Script,
}

impl NamingType {
    /// The type an explicit declaration plus a series resolve to.
    ///
    /// An undeclared doctype keeps exactly the pre-existing behaviour: a
    /// non-empty series means [`NamingType::Series`], no series means
    /// [`NamingType::Uuid`]. Declaring a type always wins.
    #[must_use]
    pub fn resolve(explicit: Option<Self>, series: &str) -> Self {
        explicit.unwrap_or(if series.is_empty() {
            Self::Uuid
        } else {
            Self::Series
        })
    }

    /// Whether the id is chosen by the engine, so the Desk shows it read-only.
    ///
    /// `Prompt` and `Field` are the two a human influences — the first by
    /// typing, the second by filling another field — so every other type is
    /// engine-owned. Written as the exception list rather than an enumeration
    /// so a future naming method is read-only by default, which is the safe
    /// default for an id the engine mints.
    #[must_use]
    pub fn is_engine_owned(self) -> bool {
        !matches!(self, Self::Prompt | Self::Field)
    }
}
