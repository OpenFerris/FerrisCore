//! Boot-time seeding of the Single `DocTypes`.
//!
//! A Single has one fixed row, named after its `DocType`, and no list. The
//! engine materialises it lazily on the first `get_single`, applying each
//! field's declared `default`, so a freshly registered Single "exists" the
//! first time the Desk opens it.
//!
//! That is enough for the Desk, but not for the host: install-wide logic (the
//! global search, the scheduler, the web shell) reads these settings *before*
//! anyone opens them in the Desk, and would otherwise have to treat "not yet
//! materialised" as a third state alongside "set" and "unset". Seeding at boot
//! collapses that state: every settings Single exists, with its defaults, the
//! moment the server can serve a request — so the host reads real information
//! rather than an empty row, and the Desk opens on the declared defaults.

use ferris_db::Db;

use crate::store::DocStore;

/// What [`seed_singletons`] did.
#[derive(Debug, Default, Clone)]
pub struct SeedReport {
    /// Names of the Singles left existing (created on this boot, or already
    /// present). In registry order.
    pub seeded: Vec<String>,
    /// `name: error` lines for Singles that could not be materialised. A bad
    /// declaration must not stop the server from booting, so these are reported
    /// for the caller to log rather than returned as an error.
    pub warnings: Vec<String>,
}

/// Materialise every registered Single `DocType`, applying its declared field
/// defaults.
///
/// Idempotent: a Single whose row already exists is only read, never
/// overwritten. Called from the server's boot sequence; safe to call again.
///
/// Never fails as a whole — a failure for one Single is collected in
/// [`SeedReport::warnings`] so one bad declaration cannot abort the boot.
pub async fn seed_singletons(pool: &Db) -> SeedReport {
    let mut report = SeedReport::default();
    for meta in ferris_schema::list_doctypes_metas() {
        if !meta.is_single {
            continue;
        }
        match DocStore::get_single_system(pool, &meta.name).await {
            Ok(_) => report.seeded.push(meta.name.clone()),
            Err(e) => report.warnings.push(format!("{}: {e}", meta.name)),
        }
    }
    report
}

/// What [`seed_reference_data`] did.
#[derive(Debug, Default, Clone)]
pub struct ReferenceSeedReport {
    /// `Module Def` rows created on this boot.
    pub modules: usize,
    /// `Language` rows created on this boot.
    pub languages: usize,
    /// `doctype: error` lines. A seed must not stop the boot.
    pub warnings: Vec<String>,
}

/// Seed the reference tables the Desk links to: one `Module Def` per module a
/// registered DocType declares, and one `Language` per advertised locale.
///
/// Both are `Link` targets (`Workspace.module`, `System Settings.language`,
/// `User.language`), so an empty table rendered every one of those pickers
/// blank and left each seeded `Workspace` pointing at a missing row. Idempotent:
/// an existing row is only checked, never overwritten — an operator may rename
/// a module or language and the seed must not undo it.
pub async fn seed_reference_data(pool: &Db, languages: &[String]) -> ReferenceSeedReport {
    let mut report = ReferenceSeedReport::default();

    let mut modules: Vec<String> = ferris_schema::list_doctypes_metas()
        .iter()
        .map(|meta| meta.module.clone())
        .filter(|module| !module.trim().is_empty())
        .collect();
    modules.sort_unstable();
    modules.dedup();

    for module in modules {
        match DocStore::exists_system(pool, "Module Def", &module).await {
            Ok(true) => {}
            Ok(false) => {
                let row = serde_json::json!({
                    "name": module,
                    "module_name": module,
                    "app_name": "",
                    "module_type": "Modules",
                });
                match DocStore::create_system(pool, "Module Def", row.into()).await {
                    Ok(_) => report.modules += 1,
                    Err(error) => report
                        .warnings
                        .push(format!("Module Def `{module}`: {error}")),
                }
            }
            Err(error) => report
                .warnings
                .push(format!("Module Def `{module}`: {error}")),
        }
    }

    for tag in languages {
        match DocStore::exists_system(pool, "Language", tag).await {
            Ok(true) => {}
            Ok(false) => {
                let row = serde_json::json!({
                    "name": tag,
                    "language_name": language_label(tag),
                    "code": tag,
                });
                match DocStore::create_system(pool, "Language", row.into()).await {
                    Ok(_) => report.languages += 1,
                    Err(error) => report.warnings.push(format!("Language `{tag}`: {error}")),
                }
            }
            Err(error) => report.warnings.push(format!("Language `{tag}`: {error}")),
        }
    }

    report
}

/// The native name of the locales the binary ships; an unknown tag is its own
/// label, which is still readable in a picker.
fn language_label(tag: &str) -> String {
    match tag {
        "en" => "English".to_string(),
        "es" => "Español".to_string(),
        "pt" => "Português".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferris_schema::{DoctypeMeta, FieldMeta};

    fn admin() -> ferris_auth::User {
        let mut user =
            ferris_auth::User::system_user("admin@local".to_string(), "Admin".to_string());
        user.roles = vec!["System Manager".to_string()];
        user
    }

    /// The reference seed creates one row per module and locale, with the name
    /// a `Link` stores, and never duplicates on a second boot.
    #[tokio::test]
    async fn reference_rows_are_seeded_once_with_their_link_names() {
        ferris_schema::doctypes::register_doctypes();
        let pool = ferris_db::memory_db().await.unwrap();

        let first = seed_reference_data(&pool, &["en".to_string(), "es".to_string()]).await;
        assert!(first.warnings.is_empty(), "{:?}", first.warnings);
        assert!(first.modules > 0, "the registry declares modules");
        assert_eq!(first.languages, 2);

        let second = seed_reference_data(&pool, &["en".to_string(), "es".to_string()]).await;
        assert_eq!(second.modules, 0, "a second boot creates nothing");
        assert_eq!(second.languages, 0);

        assert!(
            DocStore::exists_system(&pool, "Language", "en")
                .await
                .unwrap()
        );
        assert!(
            DocStore::exists_system(&pool, "Module Def", "Build")
                .await
                .unwrap()
        );
    }

    /// Seeding materialises a Single's row with its declared defaults, so host
    /// logic that reads the setting never sees a missing row.
    #[tokio::test]
    async fn seeding_materialises_singles_with_their_defaults() {
        let name = "Seed Probe Settings";
        ferris_schema::register_doctype(
            DoctypeMeta::new(name, "Test")
                .single()
                .field(FieldMeta::int("max_results").default_value("7")),
        );

        let pool = ferris_db::memory_db().await.unwrap();
        let report = seed_singletons(&pool).await;
        assert!(
            report.seeded.iter().any(|n| n == name),
            "the Single should have been seeded: {report:?}"
        );
        assert!(
            !report.warnings.iter().any(|w| w.starts_with(name)),
            "seeding reported a warning for {name}: {report:?}"
        );

        let row = DocStore::get_single_system(&pool, name).await.unwrap();
        assert_eq!(
            row.get("max_results").and_then(ferris_core::Value::as_i64),
            Some(7),
            "the default must have been applied"
        );
    }

    /// Seeding is idempotent and never clobbers a value an operator has set.
    #[tokio::test]
    async fn seeding_is_idempotent_and_keeps_saved_values() {
        let name = "Seed Probe Idempotent Settings";
        ferris_schema::register_doctype(
            DoctypeMeta::new(name, "Test")
                .single()
                .field(FieldMeta::int("max_results").default_value("7")),
        );

        let pool = ferris_db::memory_db().await.unwrap();
        seed_singletons(&pool).await;

        // Operator saves a value...
        ferris_document::with_user(Some(admin()), async {
            DocStore::save_single(&pool, name, ferris_core::value!({ "max_results": 99 }))
                .await
                .unwrap();
        })
        .await;

        // ...and a second boot keeps it.
        seed_singletons(&pool).await;
        let row = DocStore::get_single_system(&pool, name).await.unwrap();
        assert_eq!(
            row.get("max_results").and_then(ferris_core::Value::as_i64),
            Some(99),
            "seeding must not overwrite a saved value"
        );
    }
}
