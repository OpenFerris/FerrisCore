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
