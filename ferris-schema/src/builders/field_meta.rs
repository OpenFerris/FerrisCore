use crate::registry::FieldMeta;

impl FieldMeta {
    /// A new field of `fieldtype`, with only its name and type set.
    #[must_use]
    pub fn new(fieldname: impl Into<String>, fieldtype: impl Into<String>) -> Self {
        Self {
            fieldname: fieldname.into(),
            fieldtype: fieldtype.into(),
            ..Self::default()
        }
    }

    /// A `Data` (single-line text) field.
    #[must_use]
    pub fn data(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Data")
    }

    /// A `Small Text` field.
    #[must_use]
    pub fn small_text(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Small Text")
    }

    /// A `Text` (multi-line) field.
    #[must_use]
    pub fn text(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Text")
    }

    /// A `Long Text` (unbounded multi-line) field.
    #[must_use]
    pub fn long_text(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Long Text")
    }

    /// A `Text Editor` (rich text) field.
    #[must_use]
    pub fn text_editor(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Text Editor")
    }

    /// A `Code` field.
    #[must_use]
    pub fn code(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Code")
    }

    /// A `Markdown Editor` field.
    #[must_use]
    pub fn markdown(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Markdown Editor")
    }

    /// An `HTML Editor` field.
    #[must_use]
    pub fn html(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "HTML Editor")
    }

    /// An `HTML` block: not an input, renders `options` as HTML on the form
    /// (Frappe semantics — the content comes from the metadata, so only
    /// callers who may write metadata can inject markup).
    #[must_use]
    pub fn html_block(fieldname: impl Into<String>, options: impl Into<String>) -> Self {
        Self::new(fieldname, "HTML").options(options)
    }

    /// A `JSON` field.
    #[must_use]
    pub fn json(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "JSON")
    }

    /// A `Password` field.
    #[must_use]
    pub fn password(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Password")
    }

    /// A `Secret` field: encrypted at rest and recoverable by a trusted
    /// server-side context, unlike the one-way [`Self::password`].
    #[must_use]
    pub fn secret(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Secret")
    }

    /// An `Int` field.
    #[must_use]
    pub fn int(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Int")
    }

    /// A `Float` field.
    #[must_use]
    pub fn float(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Float")
    }

    /// A `Currency` field.
    #[must_use]
    pub fn currency(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Currency")
    }

    /// A `Percent` field.
    #[must_use]
    pub fn percent(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Percent")
    }

    /// A `Rating` field.
    #[must_use]
    pub fn rating(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Rating")
    }

    /// A `Check` (boolean) field.
    #[must_use]
    pub fn check(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Check")
    }

    /// An `Email` field (single-line text with email validation).
    #[must_use]
    pub fn email(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Email")
    }

    /// A `Phone` field (single-line text with phone validation).
    #[must_use]
    pub fn phone(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Phone")
    }

    /// A `URL` field (single-line text with URL validation).
    #[must_use]
    pub fn url(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "URL")
    }

    /// A `Color` field (hex colour).
    #[must_use]
    pub fn color(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Color")
    }

    /// A `Date` field.
    #[must_use]
    pub fn date(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Date")
    }

    /// A `Datetime` field.
    #[must_use]
    pub fn datetime(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Datetime")
    }

    /// A `Time` field.
    #[must_use]
    pub fn time(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Time")
    }

    /// A `Week` field (ISO week, `YYYY-Www`).
    #[must_use]
    pub fn week(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Week")
    }

    /// A `Duration` field.
    #[must_use]
    pub fn duration(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Duration")
    }

    /// A `Select` field; `options` is the newline-separated choice list.
    #[must_use]
    pub fn select(fieldname: impl Into<String>, options: impl Into<String>) -> Self {
        Self::new(fieldname, "Select").options(options)
    }

    /// A `Link` field pointing at `options` (another DocType).
    #[must_use]
    pub fn link(fieldname: impl Into<String>, options: impl Into<String>) -> Self {
        Self::new(fieldname, "Link").options(options)
    }

    /// A `Dynamic Link` field.
    #[must_use]
    pub fn dynamic_link(fieldname: impl Into<String>, options: impl Into<String>) -> Self {
        Self::new(fieldname, "Dynamic Link").options(options)
    }

    /// A `Table` (child table) field; `options` is the child DocType.
    #[must_use]
    pub fn table(fieldname: impl Into<String>, options: impl Into<String>) -> Self {
        Self::new(fieldname, "Table").options(options)
    }

    /// A `Table MultiSelect` field; `options` is the child DocType.
    #[must_use]
    pub fn table_multi_select(fieldname: impl Into<String>, options: impl Into<String>) -> Self {
        Self::new(fieldname, "Table MultiSelect").options(options)
    }

    /// An `Attach` field.
    #[must_use]
    pub fn attach(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Attach")
    }

    /// An `Attach Image` field.
    #[must_use]
    pub fn attach_image(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Attach Image")
    }

    /// A `File` field holding a file path or URL. The Desk renders it like
    /// `Attach`; the type is kept distinct in the metadata.
    #[must_use]
    pub fn file(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "File")
    }

    /// An `Image` field: renders the file another field of the same document
    /// points at (`options` is that field's name, Frappe semantics).
    #[must_use]
    pub fn image(fieldname: impl Into<String>, options: impl Into<String>) -> Self {
        Self::new(fieldname, "Image").options(options)
    }

    /// A `Signature` field: a canvas-captured signature stored as a data URL.
    #[must_use]
    pub fn signature(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Signature")
    }

    /// A `Barcode` field: the value is rendered as a barcode image.
    #[must_use]
    pub fn barcode(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Barcode")
    }

    /// A `Geolocation` field: a GeoJSON geometry/feature collection.
    #[must_use]
    pub fn geolocation(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Geolocation")
    }

    /// An `Attachment Gallery` field: a responsive gallery of the document's
    /// files. Not an input — the value lives in the attached files themselves
    /// (`options` optionally restricts by file type).
    #[must_use]
    pub fn attachment_gallery(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Attachment Gallery")
    }

    /// A layout-only `Section Break`.
    #[must_use]
    pub fn section_break(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Section Break")
    }

    /// A layout-only `Column Break`.
    #[must_use]
    pub fn column_break(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Column Break")
    }

    /// A layout-only `Tab Break`.
    #[must_use]
    pub fn tab_break(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Tab Break")
    }

    /// A caption-only `Heading` (a Ferris form extension: an unlabelled
    /// heading, not an input).
    #[must_use]
    pub fn heading(fieldname: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(fieldname, "Heading").label(label)
    }

    /// A `Button` field: renders an action button; `options` is the
    /// `/api/method/<name>` the Desk calls when it is clicked.
    #[must_use]
    pub fn button(fieldname: impl Into<String>, options: impl Into<String>) -> Self {
        Self::new(fieldname, "Button").options(options)
    }

    /// A `Read Only` field: carries a value the caller may not edit (fetch
    /// target, computed result).
    #[must_use]
    pub fn read_only_field(fieldname: impl Into<String>) -> Self {
        Self::new(fieldname, "Read Only")
    }

    /// Mark the field **virtual**: not persisted, computed on read, and
    /// read-only in the form (Frappe's Virtual DocField).
    #[must_use]
    pub fn virtual_field(mut self) -> Self {
        self.r#virtual = true;
        self
    }

    /// Mark the field **masked**: the value is masked on read unless the caller
    /// holds the `mask` permission at the field's `permlevel`
    /// (<https://docs.frappe.io/framework/data-masking>).
    #[must_use]
    pub fn masked(mut self) -> Self {
        self.mask = true;
        self
    }

    /// Replace the field label.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Mark the field required.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.reqd = true;
        self
    }

    /// Set the declared default (a string; seeded on Single materialisation).
    #[must_use]
    pub fn default_value(mut self, default: impl Into<String>) -> Self {
        self.default = Some(default.into());
        self
    }

    /// Set the options string (Select choices, Link/Table target DocType).
    #[must_use]
    pub fn options(mut self, options: impl Into<String>) -> Self {
        self.options = Some(options.into());
        self
    }

    /// Mark the field read-only.
    #[must_use]
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// Hide the field from the form.
    #[must_use]
    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    /// Set the field's permission level (Frappe's *field-level permission*).
    ///
    /// Level 0 is the document itself: any caller the document-level check
    /// admits reaches it. A level above 0 is granted only by a `DocPerm` row
    /// for one of the caller's roles that carries the right at that level, and
    /// System Manager always reaches every level. The document engine strips an
    /// unreadable field on read, ignores it on write, and masks any `Password`
    /// field for a caller who may not read it.
    #[must_use]
    pub fn permlevel(mut self, permlevel: u8) -> Self {
        self.permlevel = permlevel;
        self
    }

    /// Show the field in the list view.
    #[must_use]
    pub fn in_list_view(mut self) -> Self {
        self.in_list_view = true;
        self
    }

    /// Offer the field as a standard list filter.
    #[must_use]
    pub fn in_standard_filter(mut self) -> Self {
        self.in_standard_filter = true;
        self
    }

    /// Include the field in global search.
    #[must_use]
    pub fn in_global_search(mut self) -> Self {
        self.in_global_search = true;
        self
    }

    /// Enforce uniqueness on the field.
    ///
    /// The flag is materialized by the table DDL as a unique index: over the
    /// real column when the field has one, otherwise over a JSON expression
    /// (`json_extract(data, '$.field')` on SQLite, `data->>'field'` on
    /// Postgres, a generated column on MySQL).
    #[must_use]
    pub fn unique(mut self) -> Self {
        self.unique = true;
        self
    }

    /// Show the field only when `expression` evaluates truthy.
    #[must_use]
    pub fn depends_on(mut self, expression: impl Into<String>) -> Self {
        self.depends_on = Some(expression.into());
        self
    }

    /// Attach a help description.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the display width (fraction of the row).
    #[must_use]
    pub fn width(mut self, width: impl Into<String>) -> Self {
        self.width = Some(width.into());
        self
    }

    /// Set the numeric precision.
    #[must_use]
    pub fn precision(mut self, precision: u8) -> Self {
        self.precision = Some(precision);
        self
    }

    /// Set the input placeholder.
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set the maximum accepted length.
    #[must_use]
    pub fn max_length(mut self, max_length: u32) -> Self {
        self.max_length = Some(max_length);
        self
    }

    /// Fetch the value from `expression` (`"link_field.source_field"`).
    #[must_use]
    pub fn fetch_from(mut self, expression: impl Into<String>) -> Self {
        self.fetch_from = Some(expression.into());
        self
    }

    /// Render the field bold in the list view.
    #[must_use]
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    /// Exclude the field from copy.
    #[must_use]
    pub fn no_copy(mut self) -> Self {
        self.no_copy = true;
        self
    }
}
