//! The registry → policy → engine half of an install.

use ferris_auth::permissions::{
    DocTypeCaps, PermissionEngine, PermissionRule, Policy, PolicyStatement, doc_type_policy,
    set_global_engine,
};
use ferris_schema::list_doctypes_metas;
use tracing::{info, warn};

use super::source::{PolicyInstall, PolicySource};

/// Build the permission engine from the registry and make it the global one.
///
/// Rebuilds from scratch every time, so it is idempotent and safe to call after
/// registering more DocTypes. It does **not** fail when a DocType declares
/// nothing: default-deny means an undeclared DocType is unreadable by anyone but
/// `System Manager`, which is the documented baseline and not an error.
///
/// The Role Permissions Builder path is
/// [`install_policies_with_role_permissions`]: same rebuild, plus the stored
/// `DocPerm` rows, which this function does not see.
pub fn install_policies(source: &PolicySource) -> PolicyInstall {
    let (statements, report) = collect_registry_statements(source);
    install_statements(statements, source, report)
}

/// The registry → statements half of an install.
///
/// Kept separate from the engine half so the Role Permissions Builder can add
/// its stored rows to the same rebuild.
fn collect_registry_statements(source: &PolicySource) -> (Vec<PolicyStatement>, PolicyInstall) {
    let (rules, mut report) = collect_registry_rules(source);
    let statements = translate(rules, &mut report);
    (statements, report)
}

/// One `DocType`'s rules, together with the capabilities they translate with.
struct DocTypeRules {
    rules: Vec<PermissionRule>,
    caps: DocTypeCaps,
}

/// The registry → per-`DocType` rules half of an install.
///
/// The rules stay grouped by `DocType` (instead of becoming statements right
/// away) because a site-specific `Custom DocPerm` override has to be merged
/// **before** translation: a `select` row revokes the compiled `DocPerm` too,
/// and a statement already emitted cannot be taken back.
fn collect_registry_rules(source: &PolicySource) -> (Vec<(String, DocTypeRules)>, PolicyInstall) {
    let mut report = PolicyInstall::default();
    let mut rules = Vec::new();

    for meta in list_doctypes_metas() {
        if source.exclude.iter().any(|e| *e == meta.name) {
            continue;
        }
        report.doctypes += 1;
        if meta.permissions.is_empty() {
            continue;
        }
        report.doctypes_with_rules += 1;
        rules.push((
            meta.name.clone(),
            DocTypeRules {
                rules: meta.permissions.clone(),
                caps: DocTypeCaps {
                    submittable: meta.is_submittable,
                    allow_import: meta.allow_import,
                },
            },
        ));
    }

    (rules, report)
}

/// Translate per-`DocType` rules into the statements of one policy.
fn translate(
    rules: Vec<(String, DocTypeRules)>,
    report: &mut PolicyInstall,
) -> Vec<PolicyStatement> {
    let mut statements = Vec::new();
    for (doctype, entry) in rules {
        let policy = doc_type_policy(&doctype, &entry.rules, entry.caps);
        report.statements += policy.statements.len();
        report.skipped.extend(policy.skipped);
        statements.extend(policy.statements);
    }
    statements
}

/// The capabilities a `DocType` translates with, defaulting for an unregistered
/// one (the historical behaviour of a stored row for an unknown `DocType`).
fn caps_for(doctype: &str) -> DocTypeCaps {
    ferris_schema::get_doctype(doctype).map_or_else(DocTypeCaps::default, |meta| DocTypeCaps {
        submittable: meta.is_submittable,
        allow_import: meta.allow_import,
    })
}

/// The rule list for `doctype`, created (with its capabilities) on first use.
fn rules_entry<'a>(
    rules: &'a mut Vec<(String, DocTypeRules)>,
    index: &mut std::collections::HashMap<String, usize>,
    doctype: &str,
) -> &'a mut DocTypeRules {
    if let Some(position) = index.get(doctype) {
        return &mut rules[*position].1;
    }
    rules.push((
        doctype.to_string(),
        DocTypeRules {
            rules: Vec::new(),
            caps: caps_for(doctype),
        },
    ));
    let position = rules.len() - 1;
    index.insert(doctype.to_string(), position);
    &mut rules[position].1
}

/// Read the operator document off disk into a [`PolicySource`].
///
/// The file is optional: a missing path is not a failure, because most installs
/// will not have one and a boot-time hard error for an absent optional file is a
/// bad trade. A file that exists and does not parse *is* reported — by
/// [`install_statements`], which logs it at warn level.
fn read_policy_path(path: Option<&std::path::Path>) -> PolicySource {
    let policy_json = path.and_then(|p| match std::fs::read_to_string(p) {
        Ok(contents) => Some(contents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            warn!("authz: policy document at {} unreadable: {e}", p.display());
            None
        }
    });
    if let Some(p) = path {
        if policy_json.is_some() {
            info!("authz: policy document {}", p.display());
        }
    }
    PolicySource {
        policy_json,
        exclude: Vec::new(),
    }
}

/// The engine half of an install: one policy made of the gathered statements,
/// the operator JSON parsed on top, then the whole engine installed.
fn install_statements(
    statements: Vec<PolicyStatement>,
    source: &PolicySource,
    mut report: PolicyInstall,
) -> PolicyInstall {
    let mut engine = PermissionEngine::default();
    if !statements.is_empty() {
        engine.add_policy(Policy::new(statements));
    }

    if let Some(json) = &source.policy_json {
        match engine.load_policy_json(json) {
            Ok(()) => {
                report.json_policies = 1;
                info!("authz: loaded operator policy document");
            }
            Err(e) => {
                // A malformed policy file must not be swallowed into an engine
                // that silently has fewer rules than the operator wrote. Refusing
                // to install is the loud option; log it loudly and continue
                // *without* it, since the alternative — booting with an engine
                // nobody asked for — is worse for availability and no better for
                // safety, because the missing policies deny by default.
                warn!("authz: operator policy document rejected: {e}");
            }
        }
    }

    set_global_engine(engine);
    report
}

/// Install from the registry, plus a policy document read off disk.
///
/// The file is optional: a missing path is not a failure, because most installs
/// will not have one and a boot-time hard error for an absent optional file is a
/// bad trade. A file that exists and does not parse *is* reported — by
/// [`install_policies`], which logs it at warn level.
pub fn install_policies_from_path(path: Option<&std::path::Path>) -> PolicyInstall {
    install_policies(&read_policy_path(path))
}

/// Install from the registry **plus** the persisted Role Permissions rows
/// ([`ferris_auth::docperm::DocPermStore`]), then the operator document.
///
/// This is the boot-time install for a server that exposes the Role Permissions
/// Builder, and the reinstall behind its endpoints: an edit must become a live
/// engine change on the same rebuild that loads DocType declarations and the
/// operator document, or the three sources would drift apart. A row's DocType
/// need not be registered — `Registry` caps are used when it is, the default
/// (non-submittable, no import) caps when it is not.
///
/// [`PolicyInstall`] counts registry entries only, so rows for DocTypes that
/// are *not* in the registry add statements without inflating `doctypes`.
pub async fn install_policies_with_role_permissions(
    pool: &ferris_db::Db,
    policy_path: Option<&std::path::Path>,
) -> ferris_core::Result<PolicyInstall> {
    let source = read_policy_path(policy_path);
    let (mut rules, mut report) = collect_registry_rules(&source);
    let mut index: std::collections::HashMap<String, usize> = rules
        .iter()
        .enumerate()
        .map(|(position, (doctype, _))| (doctype.clone(), position))
        .collect();

    let store = ferris_auth::docperm::DocPermStore::new(pool.clone());
    store.init().await?;
    // Rows arrive in key order already; group them per DocType and keep that
    // order, so statement order (and therefore the skipped-rule report) is
    // deterministic across runs.
    for row in store.list_all().await? {
        rules_entry(&mut rules, &mut index, &row.doctype)
            .rules
            .push(row.to_rule());
    }
    // Site-specific `Custom DocPerm` rows override the standard ones at
    // runtime too, not only in the metadata payload, and they are merged
    // **before** translation. A `select` row revokes the role's compiled
    // `DocPerm` as well: emitting the compiled rule first and then filtering the
    // stored rows left that statement installed, so a role the operator had
    // explicitly deselected kept the right (SCH-01/SEC-10).
    for (doctype, overrides) in crate::custom_docperm::all(pool).await? {
        let entry = rules_entry(&mut rules, &mut index, &doctype);
        ferris_schema::custom_docperm::merge_rules(&mut entry.rules, overrides);
    }

    let statements = translate(rules, &mut report);
    // FRA-03: load the `User Permission` rows into the engine's source on the
    // same rebuild that installs the policy. Boot and every Role Permissions
    // save go through this function, so a site never runs with the rows present
    // and nothing reading them — the failure this finding was about.
    ferris_auth::user_permission::install(pool).await?;
    Ok(install_statements(statements, &source, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferris_auth::permissions::{
        Permission, Resource, check_permission, global_engine_ref, set_global_engine,
    };
    use ferris_db::sql::Column;

    /// The engine is process-wide, so a test that installs one must not race
    /// the `authz::tests` modules over it.
    static SERIALISE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct Restore {
        _serialised: std::sync::MutexGuard<'static, ()>,
        previous: PermissionEngine,
    }

    impl Drop for Restore {
        fn drop(&mut self) {
            set_global_engine(self.previous.clone());
        }
    }

    /// The `User Permission` snapshot is process-wide too; this clears it after
    /// the install test so the rule does not leak into the next one.
    struct ClearRules;

    impl Drop for ClearRules {
        fn drop(&mut self) {
            ferris_auth::user_permission::clear();
        }
    }

    fn engine_scope() -> Restore {
        let serialised = SERIALISE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = global_engine_ref()
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        Restore {
            _serialised: serialised,
            previous,
        }
    }

    /// FRA-03: the install loads the `User Permission` rows, so a configured
    /// restriction is live instead of being stored and never read.
    #[tokio::test]
    async fn installing_loads_the_user_permissions() {
        let _restore = engine_scope();
        let _clear = ClearRules;

        let pool = ferris_db::memory_db().await.unwrap();
        let store = ferris_auth::user_permission::UserPermissionStore::new(pool.clone());
        store.init().await.unwrap();
        store
            .upsert(&ferris_core::permissions::UserPermissionRule::new(
                "ada@example.com",
                "Customer",
                "CUST-A",
            ))
            .await
            .unwrap();

        install_policies_with_role_permissions(&pool, None)
            .await
            .unwrap();

        assert!(
            ferris_core::permissions::has_user_permissions_for("ada@example.com", "Customer"),
            "the install must publish the stored rule"
        );
        let mut user =
            ferris_core::User::system_user("ada@example.com".to_string(), "Ada".to_string());
        user.id = "ada@example.com".to_string();
        assert!(
            !check_permission(
                &user,
                &Resource::new("Customer", "CUST-B"),
                Permission::Read
            ),
            "CUST-B is outside the rule"
        );
    }

    /// SCH-01/SEC-10: a `Custom DocPerm` row with `select` is a full revocation
    /// for its role, and it must also revoke the rules the DocType *compiled*.
    ///
    /// The merge used to run over the persisted `DocPerm` rows only, after the
    /// registry rules had already become policy statements, so a role removed
    /// from the Role Permission Manager kept the compiled read.
    #[tokio::test]
    async fn a_select_custom_docperm_removes_the_compiled_rule() {
        let _restore = engine_scope();
        let doctype = "B2 Select Custom DocPerm";
        let role = "B2 Select Role";
        ferris_schema::register_doctype(ferris_schema::DoctypeMeta {
            name: doctype.to_string(),
            module: "Test".to_string(),
            permissions: vec![PermissionRule {
                role: role.to_string(),
                read: true,
                ..Default::default()
            }],
            hidden: true,
            ..Default::default()
        });

        let pool = ferris_db::memory_db().await.unwrap();
        // The real `Custom DocPerm` columns (ADR 0001): there is no `data`
        // blob. Only the columns this test exercises are needed; the reader
        // treats an absent column as its default.
        pool.ensure_table(
            "tabCustomDocPerm",
            &[
                Column::new("name", "VARCHAR(255)").not_null().primary_key(),
                Column::new("parent", "VARCHAR(255)"),
                Column::new("role", "VARCHAR(255)"),
                Column::new("permlevel", "INTEGER"),
                Column::new("select", "BOOLEAN"),
            ],
        )
        .await
        .unwrap();
        pool.execute(&ferris_db::Stmt::new(
            "INSERT INTO tabCustomDocPerm (name, parent, role, permlevel, \"select\") \
             VALUES (?, ?, ?, ?, ?)",
            vec![
                ferris_core::Value::from("row-1"),
                ferris_core::Value::from(doctype),
                ferris_core::Value::from(role),
                ferris_core::Value::from(0_i64),
                ferris_core::Value::from(true),
            ],
        ))
        .await
        .unwrap();

        install_policies_with_role_permissions(&pool, None)
            .await
            .unwrap();

        let mut user =
            ferris_core::User::system_user("sam@example.com".to_string(), "Sam".to_string());
        user.id = "sam@example.com".to_string();
        user.roles = vec![role.to_string()];
        let resource = Resource::new(doctype, "DOC-1").with_owner("ada@example.com");
        assert!(
            !check_permission(&user, &resource, Permission::Read),
            "a select Custom DocPerm must revoke the rule the DocType compiled"
        );
    }
}
