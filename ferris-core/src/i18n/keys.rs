//! Translating metadata: DocType, field, workspace and app strings.
//!
//! The Desk renders labels that are declared in three different places — the
//! schema registry, the workspace registry and an app's descriptor — and none
//! of them carries a per-locale value. Rather than widening every record with a
//! key field, the host derives the key from the identifier that is already
//! stable and unique:
//!
//! | Surface | Key |
//! |---|---|
//! | DocType label / description | `doctype.<Name>.label` / `.description` |
//! | Field label / placeholder / description | `field.<DocType>.<field>.label`, … |
//! | Workspace, folder | `workspace.<name>.label`, `workspace.folder.<name>.label` |
//! | Theme, dashboard, widget | `theme.<id>.label`, `dashboard.<id>.label`, `widget.<name>.label` |
//!
//! Identifiers are never translated — a `DocType` name, a `fieldname` or a
//! route is an internal key the engine compares, joins and stores. Only the
//! human label attached to them is looked up here, and a missing key falls back
//! to the declared text, so key text can never leak into the UI.
//!
//! An app that cannot express a key by convention (a sidebar link to its own
//! page, a method description, a website label) simply writes the key as the
//! label itself; [`MetaTranslate::literal`] translates it when the catalogue
//! has it and returns the text untouched when it does not.

use std::borrow::Cow;

use super::{I18n, Locale};

/// Longest key accepted from a package catalogue.
///
/// Keys travel in the bootstrap `<meta>` and in `GET /api/i18n`; the cap keeps a
/// hostile or generated file from turning one request into a megabyte of JSON.
pub const MAX_KEY_LEN: usize = 200;

/// Whether `key` is an acceptable catalogue key.
///
/// Keys are lookup strings, never paths: they may contain spaces and dots (a
/// `DocType` is called `Sales Order`), but no control character, no leading or
/// trailing whitespace, and no `=` (which would make a flattened key ambiguous
/// with the TOML form).
#[must_use]
pub fn is_valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_KEY_LEN
        && key.trim() == key
        && !key.contains('=')
        && !key.chars().any(char::is_control)
}

/// Catalogue key for a DocType's human label.
#[must_use]
pub fn doctype_label(doctype: &str) -> String {
    format!("doctype.{doctype}.label")
}

/// Catalogue key for a DocType's description.
#[must_use]
pub fn doctype_description(doctype: &str) -> String {
    format!("doctype.{doctype}.description")
}

/// Catalogue key for one field's human label.
#[must_use]
pub fn field_label(doctype: &str, field: &str) -> String {
    format!("field.{doctype}.{field}.label")
}

/// Catalogue key for one field's placeholder.
#[must_use]
pub fn field_placeholder(doctype: &str, field: &str) -> String {
    format!("field.{doctype}.{field}.placeholder")
}

/// Catalogue key for one field's help text.
#[must_use]
pub fn field_description(doctype: &str, field: &str) -> String {
    format!("field.{doctype}.{field}.description")
}

/// Catalogue key for a workspace's label.
#[must_use]
pub fn workspace_label(name: &str) -> String {
    format!("workspace.{name}.label")
}

/// Catalogue key for a workspace folder's label.
#[must_use]
pub fn folder_label(name: &str) -> String {
    format!("workspace.folder.{name}.label")
}

/// Catalogue key for one sidebar item of a workspace.
///
/// Items have no stable identifier of their own — a link or group is its
/// label — so the key is scoped by workspace and derived from the text. An app
/// that would rather name the key itself writes it as the label and
/// [`MetaTranslate::literal`] resolves it.
#[must_use]
pub fn workspace_item_label(workspace: &str, label: &str) -> String {
    format!("workspace.{workspace}.item.{label}.label")
}

/// Catalogue key for a theme's label.
#[must_use]
pub fn theme_label(id: &str) -> String {
    format!("theme.{id}.label")
}

/// Catalogue key for a dashboard's label.
#[must_use]
pub fn dashboard_label(id: &str) -> String {
    format!("dashboard.{id}.label")
}

/// Catalogue key for a widget definition's label.
#[must_use]
pub fn widget_label(name: &str) -> String {
    format!("widget.{name}.label")
}

/// Prefix every key an app owns for its own UI must carry.
///
/// Metadata keys (`doctype.…`, `field.…`, `workspace.…`) are exempt: they are
/// derived from identifiers the host already validated as belonging to the app.
#[must_use]
pub fn app_prefix(app: &str) -> String {
    format!("app.{app}.")
}

/// Allocates nothing when the catalogue has no entry for the key: the returned
/// value borrows the declared fallback.
#[derive(Debug, Clone, Copy)]
pub struct MetaTranslate<'a> {
    i18n: &'a I18n,
    locale: &'a Locale,
}

impl<'a> MetaTranslate<'a> {
    /// Resolve metadata for one request's locale.
    #[must_use]
    pub fn new(i18n: &'a I18n, locale: &'a Locale) -> Self {
        Self { i18n, locale }
    }

    /// The locale this translator resolves against.
    #[must_use]
    pub fn locale(&self) -> &'a Locale {
        self.locale
    }

    /// A DocType's label, falling back to `fallback` (the declared label or the
    /// humanized identifier).
    #[must_use]
    pub fn doctype_label<'b>(&self, doctype: &str, fallback: &'b str) -> Cow<'b, str>
    where
        'a: 'b,
    {
        self.pick(&doctype_label(doctype), fallback)
    }

    /// A DocType's description, if it has one.
    #[must_use]
    pub fn doctype_description<'b>(&self, doctype: &str, fallback: &'b str) -> Cow<'b, str>
    where
        'a: 'b,
    {
        self.pick(&doctype_description(doctype), fallback)
    }

    /// One field's label.
    #[must_use]
    pub fn field_label<'b>(&self, doctype: &str, field: &str, fallback: &'b str) -> Cow<'b, str>
    where
        'a: 'b,
    {
        self.pick(&field_label(doctype, field), fallback)
    }

    /// One field's placeholder.
    #[must_use]
    pub fn field_placeholder<'b>(
        &self,
        doctype: &str,
        field: &str,
        fallback: &'b str,
    ) -> Cow<'b, str>
    where
        'a: 'b,
    {
        self.pick(&field_placeholder(doctype, field), fallback)
    }

    /// One field's help text.
    #[must_use]
    pub fn field_description<'b>(
        &self,
        doctype: &str,
        field: &str,
        fallback: &'b str,
    ) -> Cow<'b, str>
    where
        'a: 'b,
    {
        self.pick(&field_description(doctype, field), fallback)
    }

    /// A workspace's label.
    #[must_use]
    pub fn workspace_label<'b>(&self, name: &str, fallback: &'b str) -> Cow<'b, str>
    where
        'a: 'b,
    {
        self.pick(&workspace_label(name), fallback)
    }

    /// A workspace folder's label.
    #[must_use]
    pub fn folder_label<'b>(&self, name: &str, fallback: &'b str) -> Cow<'b, str>
    where
        'a: 'b,
    {
        self.pick(&folder_label(name), fallback)
    }

    /// One sidebar item of a workspace.
    #[must_use]
    pub fn workspace_item_label<'b>(&self, workspace: &str, label: &'b str) -> Cow<'b, str>
    where
        'a: 'b,
    {
        self.pick(&workspace_item_label(workspace, label), label)
    }

    /// A theme's label.
    #[must_use]
    pub fn theme_label<'b>(&self, id: &str, fallback: &'b str) -> Cow<'b, str>
    where
        'a: 'b,
    {
        self.pick(&theme_label(id), fallback)
    }

    /// A dashboard's label.
    #[must_use]
    pub fn dashboard_label<'b>(&self, id: &str, fallback: &'b str) -> Cow<'b, str>
    where
        'a: 'b,
    {
        self.pick(&dashboard_label(id), fallback)
    }

    /// A widget definition's label.
    #[must_use]
    pub fn widget_label<'b>(&self, name: &str, fallback: &'b str) -> Cow<'b, str>
    where
        'a: 'b,
    {
        self.pick(&widget_label(name), fallback)
    }

    /// Translate `text` when it is itself a catalogue key.
    ///
    /// This is the escape hatch for strings whose owner cannot be derived: an
    /// app writes `app.shop.cart.empty` as the label and the catalogue decides
    /// the text. An ordinary label ("Status") is returned untouched.
    #[must_use]
    pub fn literal<'b>(&self, text: &'b str) -> Cow<'b, str>
    where
        'a: 'b,
    {
        if text.is_empty() {
            return Cow::Borrowed(text);
        }
        self.pick(text, text)
    }

    fn pick<'b>(&self, key: &str, fallback: &'b str) -> Cow<'b, str>
    where
        'a: 'b,
    {
        self.i18n
            .message(self.locale, key)
            .map_or(Cow::Borrowed(fallback), Cow::Borrowed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Catalog;

    fn locale(tag: &str) -> Locale {
        Locale::parse(tag).expect("test locale must parse")
    }

    fn i18n() -> I18n {
        let en = Catalog::from_toml(
            locale("en"),
            r#"
"doctype.Sales Order.label" = "Sales Order"
"doctype.Sales Order.description" = "Orders placed by customers"
"field.Sales Order.customer.label" = "Customer"
"workspace.users.label" = "Users"
"theme.light.label" = "Light"
"app.shop.cart.empty" = "Your cart is empty"
"#,
        )
        .expect("the fixture is valid TOML");
        let es = Catalog::from_toml(
            locale("es"),
            r#"
"doctype.Sales Order.label" = "Pedido de venta"
"field.Sales Order.customer.label" = "Cliente"
"workspace.users.label" = "Usuarios"
"theme.light.label" = "Claro"
"#,
        )
        .expect("the fixture is valid TOML");
        I18n::new(locale("en"), vec![locale("en"), locale("es")])
            .with_catalog(en)
            .with_catalog(es)
    }

    #[test]
    fn keys_follow_the_documented_convention() {
        assert_eq!(doctype_label("Sales Order"), "doctype.Sales Order.label");
        assert_eq!(
            field_label("Sales Order", "customer"),
            "field.Sales Order.customer.label"
        );
        assert_eq!(workspace_label("users"), "workspace.users.label");
        assert_eq!(folder_label("tools"), "workspace.folder.tools.label");
        assert_eq!(app_prefix("shop"), "app.shop.");
    }

    #[test]
    fn keys_reject_control_characters_and_whitespace() {
        assert!(is_valid_key("doctype.Sales Order.label"));
        assert!(is_valid_key("app.shop.cart.empty"));
        assert!(!is_valid_key(""));
        assert!(!is_valid_key(" padded"));
        assert!(!is_valid_key("two\nlines"));
        assert!(!is_valid_key(&"x".repeat(MAX_KEY_LEN + 1)));
    }

    #[test]
    fn metadata_resolves_through_the_catalogue_and_falls_back() {
        let i18n = i18n();
        let spanish = locale("es");
        let t = MetaTranslate::new(&i18n, &spanish);

        assert_eq!(
            t.doctype_label("Sales Order", "Sales Order"),
            "Pedido de venta"
        );
        assert_eq!(
            t.field_label("Sales Order", "customer", "Customer"),
            "Cliente"
        );
        assert_eq!(t.workspace_label("users", "Users"), "Usuarios");
        assert_eq!(t.theme_label("light", "Light"), "Claro");
        // No Spanish entry: the declared label wins, never the key.
        assert_eq!(
            t.doctype_description("Sales Order", "Orders placed by customers"),
            "Orders placed by customers"
        );
        // An unknown surface keeps the text it was given.
        assert_eq!(t.doctype_label("Unknown", "Unknown"), "Unknown");
    }

    #[test]
    fn literal_translates_only_known_keys() {
        let i18n = i18n();
        let english = locale("en");
        let t = MetaTranslate::new(&i18n, &english);

        assert_eq!(t.literal("app.shop.cart.empty"), "Your cart is empty");
        assert_eq!(t.literal("Status"), "Status");
        assert_eq!(t.literal(""), "");
    }

    #[test]
    fn a_regional_locale_reads_the_base_language() {
        let i18n = i18n();
        let mexican = locale("es-MX");
        let t = MetaTranslate::new(&i18n, &mexican);

        assert_eq!(
            t.doctype_label("Sales Order", "Sales Order"),
            "Pedido de venta"
        );
    }

    #[test]
    fn an_app_can_add_a_locale_the_binary_does_not_embed() {
        let pt = Catalog::from_toml(
            locale("pt"),
            r#""app.shop.cart.empty" = "O seu carrinho está vazio""#,
        )
        .expect("the fixture is valid TOML");
        let i18n = i18n().with_catalog(pt);
        assert!(i18n.supported().contains(&locale("pt")));

        let portuguese = locale("pt");
        let t = MetaTranslate::new(&i18n, &portuguese);
        assert_eq!(
            t.literal("app.shop.cart.empty"),
            "O seu carrinho está vazio"
        );
    }
}
