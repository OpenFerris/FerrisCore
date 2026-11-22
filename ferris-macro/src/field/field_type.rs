//! The field types a `#[field]` attribute can declare.
//!
//! This is the validated representation of a `#[field(type = "…")]`
//! declaration: a typo like `type = "Dat"` is a compile error instead of a
//! string no consumer understands.
//!
//! **Wire names are Frappe's, with spaces.** The Desk renderer, the layout
//! builder, the WASM descriptor and the schema builders all match
//! `"Small Text"`, `"Dynamic Link"`, `"Table MultiSelect"`… — the names of
//! https://docs.frappe.io/framework/user/en/basics/doctypes/fieldtypes.
//! `from_name` accepts exactly those canonical names and `as_str` emits them.
#![allow(clippy::upper_case_acronyms)] // `HTML`/`JSON`/`URL` are wire values.

/// Field types whose *canonical name* is the wire value Frappe uses.
///
/// `#[field(type = "...")]` accepts the canonical names verbatim, including the
/// all-caps acronyms `HTML`, `JSON` and `URL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Data,
    Link,
    Select,
    Table,
    TableMultiSelect,
    Currency,
    Float,
    Int,
    Percent,
    Check,
    Date,
    Datetime,
    Time,
    Duration,
    Week,
    Text,
    SmallText,
    LongText,
    TextEditor,
    Code,
    Markdown,
    HTML,
    HTMLEditor,
    Color,
    Image,
    File,
    Attach,
    AttachImage,
    AttachmentGallery,
    Signature,
    Barcode,
    Geolocation,
    Phone,
    Email,
    URL,
    Password,
    Secret,
    Rating,
    DynamicLink,
    JSON,
    ReadOnly,
    Button,
    Heading,
    TabBreak,
    SectionBreak,
    ColumnBreak,
}

impl FieldType {
    /// Resolve an attribute `type = "…"` name. `None` means the name is not a
    /// known field type; the caller turns that into a compile error.
    ///
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            // Canonical (Frappe) spellings.
            "Data" => Self::Data,
            "Link" => Self::Link,
            "Select" => Self::Select,
            "Table" => Self::Table,
            "Table MultiSelect" => Self::TableMultiSelect,
            "Currency" => Self::Currency,
            "Float" => Self::Float,
            "Int" => Self::Int,
            "Percent" => Self::Percent,
            "Check" => Self::Check,
            "Date" => Self::Date,
            "Datetime" => Self::Datetime,
            "Time" => Self::Time,
            "Duration" => Self::Duration,
            "Week" => Self::Week,
            "Text" => Self::Text,
            "Small Text" => Self::SmallText,
            "Long Text" => Self::LongText,
            "Text Editor" => Self::TextEditor,
            "Code" => Self::Code,
            "Markdown Editor" => Self::Markdown,
            "HTML" => Self::HTML,
            "HTML Editor" => Self::HTMLEditor,
            "Color" => Self::Color,
            "Image" => Self::Image,
            "File" => Self::File,
            "Attach" => Self::Attach,
            "Attach Image" => Self::AttachImage,
            "Attachment Gallery" => Self::AttachmentGallery,
            "Signature" => Self::Signature,
            "Barcode" => Self::Barcode,
            "Geolocation" => Self::Geolocation,
            "Phone" => Self::Phone,
            "Email" => Self::Email,
            "URL" => Self::URL,
            "Password" => Self::Password,
            "Secret" => Self::Secret,
            "Rating" => Self::Rating,
            "Dynamic Link" => Self::DynamicLink,
            "JSON" => Self::JSON,
            "Read Only" => Self::ReadOnly,
            "Button" => Self::Button,
            "Heading" => Self::Heading,
            "Tab Break" => Self::TabBreak,
            "Section Break" => Self::SectionBreak,
            "Column Break" => Self::ColumnBreak,
            _ => return None,
        })
    }

    /// The canonical (Frappe) name `FieldMeta::fieldtype` carries. This is the
    /// only spelling that reaches the wire.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Data => "Data",
            Self::Link => "Link",
            Self::Select => "Select",
            Self::Table => "Table",
            Self::TableMultiSelect => "Table MultiSelect",
            Self::Currency => "Currency",
            Self::Float => "Float",
            Self::Int => "Int",
            Self::Percent => "Percent",
            Self::Check => "Check",
            Self::Date => "Date",
            Self::Datetime => "Datetime",
            Self::Time => "Time",
            Self::Duration => "Duration",
            Self::Week => "Week",
            Self::Text => "Text",
            Self::SmallText => "Small Text",
            Self::LongText => "Long Text",
            Self::TextEditor => "Text Editor",
            Self::Code => "Code",
            Self::Markdown => "Markdown Editor",
            Self::HTML => "HTML",
            Self::HTMLEditor => "HTML Editor",
            Self::Color => "Color",
            Self::Image => "Image",
            Self::File => "File",
            Self::Attach => "Attach",
            Self::AttachImage => "Attach Image",
            Self::AttachmentGallery => "Attachment Gallery",
            Self::Signature => "Signature",
            Self::Barcode => "Barcode",
            Self::Geolocation => "Geolocation",
            Self::Phone => "Phone",
            Self::Email => "Email",
            Self::URL => "URL",
            Self::Password => "Password",
            Self::Secret => "Secret",
            Self::Rating => "Rating",
            Self::DynamicLink => "Dynamic Link",
            Self::JSON => "JSON",
            Self::ReadOnly => "Read Only",
            Self::Button => "Button",
            Self::Heading => "Heading",
            Self::TabBreak => "Tab Break",
            Self::SectionBreak => "Section Break",
            Self::ColumnBreak => "Column Break",
        }
    }
}
