//! Geography DocTypes (Desk section **GEO**): country, currency, language.

use crate::registry::{DoctypeMeta, FieldMeta};

use super::consts::GEO_MODULE;

use super::permissions::system_manager;

/// A country, with the currency it uses.
#[must_use]
pub fn country_meta() -> DoctypeMeta {
    DoctypeMeta::new("Country", GEO_MODULE)
        .description("A country and the currency it uses.")
        .title_field("country_name")
        .sort("country_name", "asc")
        .allow_delete()
        .field(
            FieldMeta::data("country_name")
                .label("Country Name")
                .in_list_view(),
        )
        .field(FieldMeta::data("code").label("Code").in_list_view())
        .field(FieldMeta::data("iso_code").label("ISO Code"))
        .field(FieldMeta::link("currency", "Currency").label("Currency"))
        .permission(system_manager())
}

/// A currency and how it renders.
#[must_use]
pub fn currency_meta() -> DoctypeMeta {
    DoctypeMeta::new("Currency", GEO_MODULE)
        .description("A currency and how it renders.")
        .title_field("currency_name")
        .sort("currency_name", "asc")
        .allow_delete()
        .field(
            FieldMeta::data("currency_name")
                .label("Currency Name")
                .in_list_view(),
        )
        .field(
            FieldMeta::data("currency_symbol")
                .label("Currency Symbol")
                .in_list_view(),
        )
        .field(FieldMeta::data("fraction").label("Fraction"))
        .field(FieldMeta::int("fraction_units").label("Fraction Units"))
        .permission(system_manager())
}

/// A language available in the Desk.
#[must_use]
pub fn language_meta() -> DoctypeMeta {
    DoctypeMeta::new("Language", GEO_MODULE)
        .description("A language available in the Desk.")
        .title_field("language_name")
        .sort("language_name", "asc")
        .allow_delete()
        .field(
            FieldMeta::data("language_name")
                .label("Language Name")
                .in_list_view(),
        )
        .field(FieldMeta::data("code").label("Code").in_list_view())
        .permission(system_manager())
}

/// A translation of a source string into one language (the shipped catalogue).
#[must_use]
pub fn translation_meta() -> DoctypeMeta {
    DoctypeMeta::new("Translation", GEO_MODULE)
        .description("A shipped translation of a source string.")
        .sort("modified", "desc")
        .allow_delete()
        .field(
            FieldMeta::link("language", "Language")
                .label("Language")
                .in_list_view(),
        )
        .field(FieldMeta::text("source_text").label("Source Text"))
        .field(FieldMeta::text("translated_text").label("Translated Text"))
        .permission(system_manager())
}
