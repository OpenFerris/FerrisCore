//! Locale negotiation and message catalogues for every Ferris surface.
//!
//! The Desk, the server-rendered pages and the embedded apps all need the same
//! three guarantees, so they live here instead of being re-invented per crate:
//!
//! * **One stable contract.** A catalogue is a flat `String -> String` map with
//!   dotted keys (`login.title`), whether the TOML writes them inline
//!   (`"login.title" = "…"`) or as nested tables (`[login] title = "…"`). The
//!   catalogs ship *inside* the binary with [`include_str!`], so a deployment
//!   cannot boot with a half-installed translation.
//! * **Deterministic fallback.** [`I18n::translate`] walks requested locale →
//!   default locale → `en` → the key itself. A missing key never panics and
//!   never blanks a label: the caller sees the key and can spot the gap.
//! * **HTTP negotiation.** [`I18n::negotiate`] and the free [`negotiate`] parse
//!   `Accept-Language` q-values, prefer an exact tag, then the same language
//!   (`es-MX` → `es`), and finally the default. The parser is strict enough to
//!   keep a hostile header from becoming a path or a lookup key.
//!
//! The embedded `en.toml`/`es.toml` pair is the baseline catalogue set. Adding
//! another language means adding a file here, not touching call sites: a
//! supported locale without a catalogue simply falls back to `en`.

use std::collections::BTreeMap;
use std::fmt;

use crate::error::{ConfigError, Error, Result};

pub mod keys;

pub use keys::MetaTranslate;

/// Longest accepted locale tag. BCP 47 allows subtags up to 8 characters, but a
/// full tag with several extensions stays well under this; the cap exists so a
/// client cannot hand us an unbounded string.
const MAX_LOCALE_LEN: usize = 35;

const EN_TOML: &str = include_str!("en.toml");
const ES_TOML: &str = include_str!("es.toml");

/// Catalogue fragments, split by Desk area so a translation change touches one
/// file per language instead of the shared baseline. Every `en` fragment has an
/// `es` twin; the parity test walks the merged catalogue, not the fragments.
const EN_FRAGMENTS: &[&str] = &[
    include_str!("en.views.toml"),
    include_str!("en.desk.toml"),
    include_str!("en.components.toml"),
    include_str!("en.ui.toml"),
    include_str!("en.meta.toml"),
];
const ES_FRAGMENTS: &[&str] = &[
    include_str!("es.views.toml"),
    include_str!("es.desk.toml"),
    include_str!("es.components.toml"),
    include_str!("es.ui.toml"),
    include_str!("es.meta.toml"),
];

/// A normalized language tag such as `es`, `es-ES` or `pt-BR`.
///
/// Construction goes through [`Locale::parse`], which lowercases the language,
/// uppercases a two-letter region and normalizes `_` to `-`. Equality therefore
/// compares canonical forms (`es_es` and `ES-ES` both become `es-ES`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Locale(String);

impl Locale {
    /// Parse and normalize a language tag.
    ///
    /// Accepts `es`, `es-ES`, `es_ES` and `pt-BR`. Surrounding whitespace is
    /// ignored. Returns `None` for an empty tag, a tag longer than 35 bytes, a
    /// non-ASCII or punctuation character other than `-`/`_`, or an empty
    /// subtag (`es--ES`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.is_empty() || raw.len() > MAX_LOCALE_LEN {
            return None;
        }
        if !raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return None;
        }

        let mut parts = raw.split(['-', '_']);
        let language = parts.next()?;
        if language.is_empty() {
            return None;
        }

        let mut normalized = String::with_capacity(raw.len());
        normalized.push_str(&language.to_ascii_lowercase());
        for part in parts {
            if part.is_empty() {
                return None;
            }
            normalized.push('-');
            if part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()) {
                // Region: `ES` from `es`.
                normalized.push_str(&part.to_ascii_uppercase());
            } else if part.len() == 4 && part.chars().all(|c| c.is_ascii_alphabetic()) {
                // Script: `Latn` from `latn`.
                let mut chars = part.chars();
                if let Some(first) = chars.next() {
                    normalized.push(first.to_ascii_uppercase());
                    normalized.push_str(&chars.as_str().to_ascii_lowercase());
                }
            } else {
                normalized.push_str(&part.to_ascii_lowercase());
            }
        }

        Some(Self(normalized))
    }

    /// The canonical tag, e.g. `es-ES`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The primary language subtag, e.g. `es` for `es-ES`.
    #[must_use]
    pub fn language(&self) -> &str {
        self.0.find('-').map_or(&self.0, |end| &self.0[..end])
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Default for Locale {
    /// English: the catalogue that is always embedded and the last fallback of
    /// every lookup, so it is the only locale that is always valid.
    fn default() -> Self {
        Self("en".to_string())
    }
}

/// One locale's flattened messages.
#[derive(Debug, Clone)]
pub struct Catalog {
    locale: Locale,
    messages: BTreeMap<String, String>,
}

impl Catalog {
    /// Build a catalogue from TOML text.
    ///
    /// Both shapes flatten to the same dotted keys:
    ///
    /// ```toml
    /// "login.title" = "Ferris Desk"
    ///
    /// [login]
    /// title = "Ferris Desk"
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] with [`ConfigError::Invalid`] when the text is
    /// not valid TOML, when the root is not a table, or when a leaf is not a
    /// string.
    pub fn from_toml(locale: Locale, text: &str) -> Result<Self> {
        let table: toml::Table = text.parse().map_err(|error: toml::de::Error| {
            Error::Config(ConfigError::Invalid {
                key: locale.as_str().to_string(),
                msg: format!("invalid catalogue TOML: {error}"),
            })
        })?;

        let mut messages = BTreeMap::new();
        flatten_table("", &table, &mut messages)?;

        Ok(Self { locale, messages })
    }

    /// The catalogue's locale.
    #[must_use]
    pub fn locale(&self) -> &Locale {
        &self.locale
    }

    /// Look a key up in this catalogue only, without fallback.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.messages.get(key).map(String::as_str)
    }

    /// Every `(key, message)` pair, ordered by key.
    ///
    /// For validation and diagnostics: the runtime lookups go through
    /// [`Catalog::get`].
    pub fn messages(&self) -> impl Iterator<Item = (&str, &str)> {
        self.messages
            .iter()
            .map(|(key, message)| (key.as_str(), message.as_str()))
    }

    /// Number of keys in this catalogue.
    #[must_use]
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Whether this catalogue has no messages.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Merge a fragment into this catalogue. A later fragment wins on a
    /// duplicate key, which is what lets the split files evolve independently
    /// without a silent conflict in the baseline.
    fn extend(&mut self, other: Self) {
        self.messages.extend(other.messages);
    }
}

/// Flatten nested TOML tables into `a.b.c -> value`, rejecting non-string leaves.
fn flatten_table(
    prefix: &str,
    table: &toml::Table,
    out: &mut BTreeMap<String, String>,
) -> Result<()> {
    for (key, value) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };

        match value {
            toml::Value::Table(nested) => flatten_table(&path, nested, out)?,
            toml::Value::String(message) => {
                out.insert(path, message.clone());
            }
            _ => {
                return Err(Error::Config(ConfigError::Invalid {
                    key: path,
                    msg: "catalogue values must be strings".to_string(),
                }));
            }
        }
    }

    Ok(())
}

/// The process-wide catalogue set and locale policy.
///
/// [`I18n::new`] always embeds `en` (the ultimate fallback) and `es`, then
/// appends the configured locales. A locale without an embedded catalogue is
/// still negotiable; it just resolves every key through the fallback chain.
#[derive(Debug, Clone)]
pub struct I18n {
    default: Locale,
    supported: Vec<Locale>,
    catalogs: BTreeMap<String, Catalog>,
}

impl I18n {
    /// Build the catalogue set.
    ///
    /// `supported` is de-duplicated and augmented with `en` (always) and with
    /// `default` when the caller omitted it. `en` is kept first so
    /// [`I18n::available_json`] is stable regardless of input order.
    #[must_use]
    pub fn new(default: Locale, supported: Vec<Locale>) -> Self {
        let mut catalogs = BTreeMap::new();

        let en = Locale("en".to_string());
        let mut en_catalog = embedded_catalog(en.clone(), EN_TOML);
        for fragment in EN_FRAGMENTS {
            en_catalog.extend(embedded_catalog(en.clone(), fragment));
        }
        catalogs.insert(en.as_str().to_string(), en_catalog);

        let es = Locale("es".to_string());
        let mut es_catalog = embedded_catalog(es.clone(), ES_TOML);
        for fragment in ES_FRAGMENTS {
            es_catalog.extend(embedded_catalog(es.clone(), fragment));
        }
        catalogs.insert(es.as_str().to_string(), es_catalog);

        let mut supported_locales = Vec::with_capacity(supported.len() + 2);
        supported_locales.push(en);
        for locale in supported {
            if !supported_locales.contains(&locale) {
                supported_locales.push(locale);
            }
        }
        if !supported_locales.contains(&default) {
            supported_locales.push(default.clone());
        }

        Self {
            default,
            supported: supported_locales,
            catalogs,
        }
    }

    /// The fallback locale used when negotiation finds nothing.
    #[must_use]
    pub fn default_locale(&self) -> &Locale {
        &self.default
    }

    /// Locales this instance can negotiate, `en` first.
    #[must_use]
    pub fn supported(&self) -> &[Locale] {
        &self.supported
    }

    /// Pick the best locale for an `Accept-Language` header.
    ///
    /// `None` or an empty header yields the default. See the free
    /// [`negotiate`] for the matching rules.
    #[must_use]
    pub fn negotiate(&self, accept_language: Option<&str>) -> Locale {
        accept_language.map_or_else(
            || self.default.clone(),
            |header| negotiate(header, &self.supported, &self.default),
        )
    }

    /// Add a catalogue layer from outside the binary (a site or an installed
    /// app) and advertise its locale.
    ///
    /// The layer is merged over whatever this instance already holds for that
    /// locale, so a missing key keeps the shipped translation instead of
    /// emptying the catalogue. A locale added this way becomes negotiable,
    /// which is what lets an app ship a language the binary does not embed.
    #[must_use]
    pub fn with_catalog(mut self, catalog: Catalog) -> Self {
        let tag = catalog.locale().clone();
        match self.catalogs.get_mut(tag.as_str()) {
            Some(existing) => existing.extend(catalog),
            None => {
                self.catalogs.insert(tag.as_str().to_string(), catalog);
            }
        }
        if !self.supported.contains(&tag) {
            self.supported.push(tag);
        }
        self
    }

    /// Look a key up without the default/`en` chain, so a caller that supplies
    /// its own fallback can tell "translated" from "not".
    ///
    /// An exact tag wins; `es-MX` falls back to the `es` catalogue before the
    /// caller's fallback, matching what negotiation promised.
    #[must_use]
    pub fn message(&self, locale: &Locale, key: &str) -> Option<&str> {
        self.lookup(locale, key).or_else(|| {
            if locale.language() == locale.as_str() {
                return None;
            }
            self.catalogs
                .get(locale.language())
                .and_then(|catalog| catalog.get(key))
        })
    }

    /// Translate `key`, falling back requested locale → default → `en` → key.
    ///
    /// The returned slice borrows either the catalogue or `key` itself; tying
    /// both to the same lifetime is what lets an unknown key be returned
    /// without allocating or leaking it.
    #[must_use]
    pub fn translate<'a>(&'a self, locale: &Locale, key: &'a str) -> &'a str {
        self.message(locale, key)
            .or_else(|| self.lookup(&self.default, key))
            .or_else(|| self.lookup_en(key))
            .unwrap_or(key)
    }

    /// Translate `key` and replace `{name}` placeholders from `args`.
    ///
    /// Unknown placeholders are preserved verbatim so a missing argument shows
    /// up in the output instead of silently deleting text. Repeated
    /// placeholders are all replaced.
    #[must_use]
    pub fn translate_with(&self, locale: &Locale, key: &str, args: &[(&str, &str)]) -> String {
        interpolate(self.translate(locale, key), args)
    }

    /// The effective messages for `locale`: `en` overlaid with that locale.
    ///
    /// This is what a client-side translator should load: it can answer every
    /// key without a second round trip, while the requested locale still wins
    /// on the keys it actually translates.
    #[must_use]
    pub fn catalog_json(&self, locale: &Locale) -> serde_json::Value {
        let mut merged: BTreeMap<&str, &str> = BTreeMap::new();
        if let Some(catalog) = self.catalogs.get("en") {
            for (key, value) in &catalog.messages {
                merged.insert(key.as_str(), value.as_str());
            }
        }
        // A regional request (`es-MX`) reads the base language before the exact
        // tag, the same order `translate` and negotiation use.
        if locale.language() != locale.as_str()
            && let Some(catalog) = self.catalogs.get(locale.language())
        {
            for (key, value) in &catalog.messages {
                merged.insert(key.as_str(), value.as_str());
            }
        }
        if let Some(catalog) = self.catalogs.get(locale.as_str()) {
            for (key, value) in &catalog.messages {
                merged.insert(key.as_str(), value.as_str());
            }
        }

        let object = merged
            .into_iter()
            .map(|(key, value)| {
                (
                    key.to_string(),
                    serde_json::Value::String(value.to_string()),
                )
            })
            .collect();
        serde_json::Value::Object(object)
    }

    /// The locale policy as JSON: `{"default":"es","supported":["en","es"]}`.
    #[must_use]
    pub fn available_json(&self) -> serde_json::Value {
        let mut object = serde_json::Map::new();
        object.insert(
            "default".to_string(),
            serde_json::Value::String(self.default.as_str().to_string()),
        );
        object.insert(
            "supported".to_string(),
            serde_json::Value::Array(
                self.supported
                    .iter()
                    .map(|locale| serde_json::Value::String(locale.as_str().to_string()))
                    .collect(),
            ),
        );
        serde_json::Value::Object(object)
    }

    fn lookup(&self, locale: &Locale, key: &str) -> Option<&str> {
        self.catalogs
            .get(locale.as_str())
            .and_then(|catalog| catalog.get(key))
    }

    fn lookup_en(&self, key: &str) -> Option<&str> {
        self.catalogs.get("en").and_then(|catalog| catalog.get(key))
    }
}

impl Default for I18n {
    /// English only. Used where no config exists yet (tests, the SSR route-list
    /// render at startup); the server always replaces it with the configured
    /// policy before serving.
    fn default() -> Self {
        Self::new(Locale::default(), vec![Locale::default()])
    }
}

/// Parse a catalogue that ships with the binary.
///
/// The files are compile-time constants and `tests::embedded_catalogues_parse`
/// proves they are valid; an empty catalogue is only a defensive fallback so a
/// future edit cannot turn a translation bug into a boot failure.
fn embedded_catalog(locale: Locale, text: &str) -> Catalog {
    Catalog::from_toml(locale.clone(), text).unwrap_or_else(|_| Catalog {
        locale,
        messages: BTreeMap::new(),
    })
}

/// Substitute `{name}` placeholders, leaving unknown ones intact.
fn interpolate(template: &str, args: &[(&str, &str)]) -> String {
    let mut output = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(open) = rest.find('{') {
        output.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];

        let Some(close) = after_open.find('}') else {
            // Unbalanced `{`: keep the remainder exactly as written.
            output.push_str(&rest[open..]);
            return output;
        };

        let name = &after_open[..close];
        if let Some((_, value)) = args.iter().find(|(candidate, _)| *candidate == name) {
            output.push_str(value);
        } else {
            output.push('{');
            output.push_str(name);
            output.push('}');
        }
        rest = &after_open[close + 1..];
    }

    output.push_str(rest);
    output
}

/// Negotiate a locale from an `Accept-Language` header.
///
/// Ranges are sorted by descending q-value (ties keep header order). For each
/// range we try an exact normalized tag first, then a match on the primary
/// language, so `es-MX` selects `es`. `q=0` ranges are refused; `*` selects
/// `default`. If nothing matches, `default` is returned.
#[must_use]
pub fn negotiate(accept_language: &str, supported: &[Locale], default: &Locale) -> Locale {
    let mut ranges: Vec<(f32, usize, Locale)> = Vec::new();

    for (index, raw) in accept_language.split(',').enumerate() {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }

        let mut pieces = raw.split(';');
        let tag = pieces.next().unwrap_or("").trim();
        let mut quality = 1.0_f32;
        for parameter in pieces {
            if let Some((name, value)) = parameter.trim().split_once('=') {
                if name.trim().eq_ignore_ascii_case("q") {
                    quality = value.trim().parse().unwrap_or(0.0);
                }
            }
        }
        if quality <= 0.0 {
            continue;
        }

        // `*` needs no entry: every path ends at `default` anyway.
        if tag != "*" {
            if let Some(locale) = Locale::parse(tag) {
                ranges.push((quality, index, locale));
            }
        }
    }

    ranges.sort_by(|left, right| right.0.total_cmp(&left.0).then(left.1.cmp(&right.1)));

    for (_, _, wanted) in &ranges {
        // Per range: an exact tag wins, then the same primary language. Only
        // if this range cannot be honoured do we move to the next q-value.
        if let Some(hit) = supported.iter().find(|candidate| *candidate == wanted) {
            return hit.clone();
        }
        if let Some(hit) = supported
            .iter()
            .find(|candidate| candidate.language() == wanted.language())
        {
            return hit.clone();
        }
    }

    default.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locale(tag: &str) -> Locale {
        Locale::parse(tag).expect("test locale must parse")
    }

    fn i18n() -> I18n {
        I18n::new(locale("en"), vec![locale("en"), locale("es")])
    }

    /// A catalogue set with placeholders, kept local to the interpolation test
    /// so the shipped Desk strings stay placeholder-free.
    fn interpolating_i18n() -> I18n {
        let mut catalogs = BTreeMap::new();
        catalogs.insert(
            "en".to_string(),
            Catalog {
                locale: locale("en"),
                messages: BTreeMap::from([(
                    "greeting".to_string(),
                    "Hello {name}, you are {role}".to_string(),
                )]),
            },
        );
        catalogs.insert(
            "es".to_string(),
            Catalog {
                locale: locale("es"),
                messages: BTreeMap::from([(
                    "greeting".to_string(),
                    "Hola {name}, eres {role}".to_string(),
                )]),
            },
        );
        I18n {
            default: locale("en"),
            supported: vec![locale("en"), locale("es")],
            catalogs,
        }
    }

    #[test]
    fn locale_normalizes_language_and_region() {
        assert_eq!(locale("es").as_str(), "es");
        assert_eq!(locale("es-ES").as_str(), "es-ES");
        assert_eq!(locale("es_ES").as_str(), "es-ES");
        assert_eq!(locale("ES-es").as_str(), "es-ES");
        assert_eq!(locale("pt-BR").as_str(), "pt-BR");
        assert_eq!(locale("  es-mx  ").as_str(), "es-MX");
        assert_eq!(locale("zh-hant").as_str(), "zh-Hant");
    }

    #[test]
    fn locale_language_strips_region() {
        assert_eq!(locale("es-ES").language(), "es");
        assert_eq!(locale("es").language(), "es");
        assert_eq!(locale("pt-BR").language(), "pt");
    }

    #[test]
    fn locale_rejects_invalid_tags() {
        assert!(Locale::parse("").is_none());
        assert!(Locale::parse("   ").is_none());
        assert!(Locale::parse("es ES").is_none());
        assert!(Locale::parse("es/ES").is_none());
        assert!(Locale::parse("españa").is_none());
        assert!(Locale::parse("es--ES").is_none());
        assert!(Locale::parse(&"a".repeat(MAX_LOCALE_LEN + 1)).is_none());
        assert!(Locale::parse(&"a".repeat(MAX_LOCALE_LEN)).is_some());
    }

    #[test]
    fn locale_display_matches_canonical_form() {
        assert_eq!(locale("es_es").to_string(), "es-ES");
    }

    #[test]
    fn negotiation_honours_q_values() {
        let supported = [locale("en"), locale("es")];
        let default = locale("en");

        assert_eq!(
            negotiate("fr;q=0.9, es;q=0.8, en;q=0.7", &supported, &default),
            locale("es")
        );
        assert_eq!(
            negotiate("es;q=0.2, en;q=0.9", &supported, &default),
            locale("en")
        );
        assert_eq!(
            negotiate("es;q=0, fr;q=0.5", &supported, &default),
            locale("en")
        );
    }

    #[test]
    fn negotiation_prefers_exact_then_language_prefix() {
        let supported = [locale("en"), locale("es")];

        assert_eq!(
            negotiate("es-MX, en;q=0.5", &supported, &locale("en")),
            locale("es")
        );
        assert_eq!(
            negotiate("en-US, es;q=0.5", &supported, &locale("es")),
            locale("en")
        );
    }

    #[test]
    fn negotiation_falls_back_to_default() {
        let supported = [locale("en"), locale("es")];
        let default = locale("es");

        assert_eq!(negotiate("de-DE, fr;q=0.4", &supported, &default), default);
        assert_eq!(negotiate("", &supported, &default), default);
        assert_eq!(negotiate("*", &supported, &default), default);
    }

    #[test]
    fn i18n_new_always_includes_en_and_default() {
        let i18n = I18n::new(locale("es"), vec![locale("es")]);
        assert_eq!(i18n.supported(), [locale("en"), locale("es")]);
        assert_eq!(i18n.default_locale(), &locale("es"));

        let i18n = I18n::new(locale("fr"), Vec::new());
        assert!(i18n.supported().contains(&locale("en")));
        assert!(i18n.supported().contains(&locale("fr")));
    }

    #[test]
    fn translate_uses_fallback_chain() {
        let i18n = i18n();

        assert_eq!(i18n.translate(&locale("es"), "login.submit"), "Entrar");
        assert_eq!(i18n.translate(&locale("en"), "login.submit"), "Sign in");
        // `fr` has no catalogue: the default (`en`) answers.
        assert_eq!(i18n.translate(&locale("fr"), "login.submit"), "Sign in");
        // Unknown keys survive as the key itself.
        assert_eq!(i18n.translate(&locale("es"), "missing.key"), "missing.key");
    }

    #[test]
    fn translate_with_interpolates_known_placeholders() {
        let i18n = interpolating_i18n();

        assert_eq!(
            i18n.translate_with(
                &locale("en"),
                "greeting",
                &[("name", "Ada"), ("role", "admin")]
            ),
            "Hello Ada, you are admin"
        );
        assert_eq!(
            i18n.translate_with(&locale("es"), "greeting", &[("name", "Ada")]),
            "Hola Ada, eres {role}"
        );
        assert_eq!(
            i18n.translate_with(&locale("es"), "missing.key", &[("name", "Ada")]),
            "missing.key"
        );
    }

    #[test]
    fn catalogue_accepts_dotted_and_nested_keys() {
        let text = r#"
            "login.title" = "Ferris Desk"
            [common]
            save = "Save"
            [login]
            submit = "Sign in"
        "#;
        let catalog = Catalog::from_toml(locale("en"), text).expect("valid catalogue");

        assert_eq!(catalog.get("login.title"), Some("Ferris Desk"));
        assert_eq!(catalog.get("login.submit"), Some("Sign in"));
        assert_eq!(catalog.get("common.save"), Some("Save"));
        assert_eq!(catalog.locale(), &locale("en"));
    }

    #[test]
    fn catalogue_rejects_non_string_leaves() {
        let error = Catalog::from_toml(locale("en"), "[login]\ntitle = 42\n")
            .expect_err("numbers are not messages");
        match error {
            Error::Config(ConfigError::Invalid { key, .. }) => assert_eq!(key, "login.title"),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn catalogue_rejects_invalid_toml() {
        let error = Catalog::from_toml(locale("en"), "[login\ntitle = \"x\"\n")
            .expect_err("unterminated tables are invalid");
        assert!(matches!(error, Error::Config(ConfigError::Invalid { .. })));
    }

    #[test]
    fn embedded_catalogues_parse() {
        let en = Catalog::from_toml(locale("en"), EN_TOML).expect("en.toml is valid");
        let es = Catalog::from_toml(locale("es"), ES_TOML).expect("es.toml is valid");

        assert_eq!(en.locale(), &locale("en"));
        assert_eq!(es.locale(), &locale("es"));
    }

    #[test]
    fn catalogues_share_keys() {
        let i18n = i18n();
        let en = i18n.catalogs.get("en").expect("en catalogue");
        let es = i18n.catalogs.get("es").expect("es catalogue");

        let en_keys: Vec<&str> = en.messages.keys().map(String::as_str).collect();
        let es_keys: Vec<&str> = es.messages.keys().map(String::as_str).collect();
        assert_eq!(en_keys, es_keys, "en.toml and es.toml must stay in sync");
    }

    /// The auth hardening added fields to `User` and `User Invitation`; every
    /// visible field needs a label in both catalogues, and the replaced
    /// `expires_on` key must not linger.
    #[test]
    fn auth_hardening_labels_are_present() {
        let labels = [
            "field.User.mfa_last_step.label",
            "field.User.oauth_subject.label",
            "field.User.email_verified.label",
            "field.User.email_verification_key.label",
            "field.User.email_verification_expires_at.label",
            "field.User.reset_password_expires_at.label",
            "field.User Invitation.token_hash.label",
            "field.User Invitation.expires_at.label",
            "field.User Invitation.invited_by.label",
            "field.User Invitation.accepted_at.label",
        ];
        let i18n = i18n();
        for tag in ["en", "es"] {
            let catalog = i18n.catalogs.get(tag).expect("catalogue");
            for key in labels {
                let value = catalog.get(key).unwrap_or_else(|| {
                    panic!("{tag}.meta.toml is missing {key}");
                });
                assert!(!value.is_empty(), "{tag}.meta.toml has an empty {key}");
            }
            assert!(
                catalog
                    .get("field.User Invitation.expires_on.label")
                    .is_none(),
                "{tag}.meta.toml still declares the orphan expires_on label"
            );
        }
    }

    #[test]
    fn required_desk_keys_are_present() {
        let required = [
            "login.title",
            "login.subtitle",
            "login.email",
            "login.password",
            "login.submit",
            "login.mfa_title",
            "login.mfa_hint",
            "login.mfa_code",
            "login.mfa_verify",
            "login.error.empty_credentials",
            "login.error.empty_code",
            "login.oauth_continue",
            "common.cancel",
            "common.save",
            "common.close",
            "common.loading",
            "common.error",
            "nav.desk",
            "nav.login",
            "nav.logout",
            "nav.workspaces",
            "nav.settings",
            "error.not_found_title",
            "error.not_found_body",
            "error.unauthorized_title",
            "error.unauthorized_body",
        ];
        let i18n = i18n();
        for key in required {
            assert!(
                i18n.catalogs
                    .get("en")
                    .and_then(|catalog| catalog.get(key))
                    .is_some(),
                "en.toml is missing {key}"
            );
            assert!(
                i18n.catalogs
                    .get("es")
                    .and_then(|catalog| catalog.get(key))
                    .is_some(),
                "es.toml is missing {key}"
            );
        }
    }

    #[test]
    fn catalog_json_includes_en_fallback_and_locale_overrides() {
        let mut catalogs = BTreeMap::new();
        catalogs.insert(
            "en".to_string(),
            Catalog {
                locale: locale("en"),
                messages: BTreeMap::from([
                    ("only_en".to_string(), "English".to_string()),
                    ("both".to_string(), "English".to_string()),
                ]),
            },
        );
        catalogs.insert(
            "es".to_string(),
            Catalog {
                locale: locale("es"),
                messages: BTreeMap::from([("both".to_string(), "Español".to_string())]),
            },
        );
        let i18n = I18n {
            default: locale("es"),
            supported: vec![locale("en"), locale("es")],
            catalogs,
        };

        let json = i18n.catalog_json(&locale("es"));
        assert_eq!(json["only_en"], "English");
        assert_eq!(json["both"], "Español");
    }

    #[test]
    fn available_json_reports_default_and_supported() {
        let i18n = I18n::new(locale("es"), Vec::new());
        let json = i18n.available_json();

        assert_eq!(json["default"], "es");
        assert_eq!(json["supported"], serde_json::json!(["en", "es"]));
    }
}
