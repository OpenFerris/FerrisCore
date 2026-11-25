//! Site-specific `Custom DocPerm` overrides.
//!
//! Frappe's **`Custom DocPerm`** is the tenant-specific permission layer: the
//! standard `DocPerm` rows ship with the app, and a Customize/Role Permission
//! edit stores an override here
//! (<https://docs.frappe.io/framework/user/en/basics/doctypes/customize>).
//!
//! Two readers must agree about a row, or the Desk and the engine will grant
//! different things:
//!
//! * `effective_meta` folds the overrides into the metadata payload;
//! * `install_policies_with_role_permissions` folds them into the runtime
//!   policy.
//!
//! Both go through [`merge_rules`] and [`rule_from_row`], the only two places
//! that decide what an override means.

// `ferris_core::Error` is the shared taxonomy; boxing it would make every
// caller match on a Box it never inspects.
#![allow(clippy::result_large_err)]

use crate::DoctypeMeta;
use ferris_core::permissions::PermissionRule;
use ferris_core::{Error, Result, SerializationError, ValidationError, Value};
use ferris_db::sql::Ident;
use ferris_db::{Db, Row, Stmt};

/// Whether `user` holds the custom permission `perm_type` on `doctype`.
///
/// Frappe's custom permissions
/// (<https://docs.frappe.io/framework/permission-types>) are assigned to roles
/// and checked like any other right. The assignment is stored on the effective
/// `Custom DocPerm` rule (`custom_permissions`), so this resolves the **same**
/// metadata the Desk receives: a permission shown as granted and one enforced
/// here cannot disagree.
///
/// The check is DocType-level. An `if_owner` rule only applies to a document
/// the caller owns, which this signature cannot know, so such a rule is not
/// treated as a blanket grant; a document-level caller should check the rule it
/// already resolved instead of widening it here.
///
/// # Errors
///
/// A malformed `Custom DocPerm` row or a database failure.
pub async fn has_custom_permission(
    pool: &Db,
    user: &ferris_core::User,
    doctype: &str,
    perm_type: &str,
) -> Result<bool> {
    has_custom_permission_on(pool, user, doctype, None, perm_type).await
}

/// [`has_custom_permission`] for one document, including `if_owner` rules.
///
/// A DocType-level check cannot answer an `if_owner` grant (it needs the
/// stored owner), so the plain form deliberately skips those rules. A caller
/// that already loaded the document passes `owner` here and gets Frappe's full
/// semantics.
///
/// # Errors
///
/// A malformed `Custom DocPerm` row or a database failure.
pub async fn has_custom_permission_on(
    pool: &Db,
    user: &ferris_core::User,
    doctype: &str,
    owner: Option<&str>,
    perm_type: &str,
) -> Result<bool> {
    if user.has_role(ferris_core::permissions::SYSTEM_MANAGER_ROLE) {
        return Ok(true);
    }
    let Some(rules) = effective_permissions(pool, doctype).await? else {
        return Ok(false);
    };
    Ok(rules.iter().any(|rule| {
        let role_matches = rule.role == ferris_core::permissions::ALL_USER_ROLE
            || user.roles.iter().any(|role| role == &rule.role);
        let owner_matches =
            !rule.if_owner || owner.is_some_and(|owner| !owner.is_empty() && owner == user.id);
        rule.permlevel == 0 && role_matches && owner_matches && rule.grants_custom(perm_type)
    }))
}

/// Merge stored `Custom DocPerm` rows over `meta.permissions`.
///
/// # Errors
///
/// A malformed stored row or a database failure.
///
/// A row for the same `(role, permlevel)` replaces the standard rule; a row
/// with `select` also drops the standard rows the same pair would otherwise
/// keep. Malformed `custom_permissions` JSON is a serialization error, never a
/// silently empty grant.
pub async fn apply(pool: &Db, doctype: &str, meta: &mut DoctypeMeta) -> Result<()> {
    let overrides = for_doctype(pool, doctype).await?;
    merge_rules(&mut meta.permissions, overrides);
    Ok(())
}

/// The `Custom DocPerm` overrides of **one** DocType.
///
/// [`all`] is the right reader only when every row is needed (the runtime
/// policy install). Everything that resolves a single DocType — the effective
/// metadata of a form, `has_custom_permission`, the role matrix — goes through
/// this one: the table is indexed by `parent`, and a site with many overrides
/// should not decode all of them to find one.
///
/// # Errors
///
/// A malformed stored row or a database failure.
pub async fn for_doctype(pool: &Db, doctype: &str) -> Result<Vec<(PermissionRule, bool)>> {
    let mut rules = Vec::new();
    let Some(rows) = rows_for(pool, Some(doctype)).await? else {
        return Ok(rules);
    };
    for row in rows {
        rules.push(rule_from_row(&row)?);
    }
    Ok(rules)
}

/// `SELECT *` over `Custom DocPerm`, optionally narrowed to one `parent`.
///
/// `Ok(None)` means the table does not exist yet, which the callers read as
/// "no overrides" rather than as a failure.
async fn rows_for(pool: &Db, doctype: Option<&str>) -> Result<Option<Vec<Row>>> {
    const TABLE: &str = "tabCustomDocPerm";
    if !pool.table_exists(TABLE).await? {
        return Ok(None);
    }
    let quoted = Ident::new(TABLE)?.quote(pool.dialect());
    let stmt = match doctype {
        Some(doctype) => Stmt::new(
            format!("SELECT * FROM {quoted} WHERE parent = ? ORDER BY name"),
            vec![Value::from(doctype)],
        ),
        None => Stmt::new(format!("SELECT * FROM {quoted} ORDER BY name"), Vec::new()),
    };
    Ok(Some(pool.fetch_all(&stmt).await?))
}

/// Fold `Custom DocPerm` overrides into a permission list.
///
/// One implementation for the metadata payload, the lightweight permission
/// lookup [`effective_permissions`] **and** the runtime policy install
/// (`ferris-api`'s `authz::install`), so the three cannot disagree.
///
/// This must run over the *complete* rule list for a `DocType` — compiled
/// `DocPerm` rows and persisted ones — before any statement is emitted. A
/// `select` row is a full revocation for its role; merging it after the
/// compiled rules had already become policy statements left those statements
/// in place, which is exactly the fail-open the `select` flag exists to close.
pub fn merge_rules(rules: &mut Vec<PermissionRule>, overrides: Vec<(PermissionRule, bool)>) {
    for (rule, select) in overrides {
        if select {
            // A `select` row is a full replacement: it supersedes every
            // standard row for the role, not just the same permlevel.
            rules.retain(|existing| existing.role != rule.role);
        } else {
            rules.retain(|existing| {
                !(existing.role == rule.role && existing.permlevel == rule.permlevel)
            });
        }
        rules.push(rule);
    }
}

/// The effective **permissions** of a DocType, without loading custom fields or
/// property setters.
///
/// # Errors
///
/// A malformed stored row or a database failure.
///
/// `has_custom_permission` runs on action checks; going through
/// [`effective_meta`] there would initialise the whole customization store for
/// a question that only needs the rule list. This is the same merge, with the
/// same override semantics, over the registry rules alone.
pub async fn effective_permissions(
    pool: &Db,
    doctype: &str,
) -> Result<Option<Vec<PermissionRule>>> {
    let Some(meta) = crate::get_doctype(doctype) else {
        return Ok(None);
    };
    let mut rules = meta.permissions.clone();
    merge_rules(&mut rules, for_doctype(pool, doctype).await?);
    Ok(Some(rules))
}

/// Every `Custom DocPerm` row, grouped by the DocType it overrides.
///
/// # Errors
///
/// A malformed stored row or a database failure.
///
/// The policy install uses this to fold the overrides into the *runtime*
/// engine, not only into the metadata payload; `effective_meta` uses the
/// per-DocType query below. Both share [`rule_from_row`], so the two readers
/// cannot disagree about what a row means.
///
/// The row is read as its real columns (ADR 0001): there is no `data` blob to
/// decode. `SELECT *` keeps the reader working over the subset a test or an
/// older install created; a missing right column reads as its default, while a
/// missing `parent` is a schema mismatch and fails instead of dropping every
/// override.
pub async fn all(
    pool: &Db,
) -> Result<std::collections::HashMap<String, Vec<(PermissionRule, bool)>>> {
    let mut overrides = std::collections::HashMap::new();
    let Some(rows) = rows_for(pool, None).await? else {
        return Ok(overrides);
    };
    for row in rows {
        if !row.has("parent") {
            return Err(Error::Validation(ValidationError::Invalid(
                "tabCustomDocPerm has no `parent` column".to_string(),
            )));
        }
        let Some(parent) = row.opt_str("parent") else {
            continue;
        };
        let parent = parent.trim();
        if parent.is_empty() {
            continue;
        }
        let (rule, select) = rule_from_row(&row)?;
        overrides
            .entry(parent.to_string())
            .or_insert_with(Vec::new)
            .push((rule, select));
    }
    Ok(overrides)
}

/// Decode one `Custom DocPerm` row into a rule plus its `select` flag.
///
/// Every value comes from its typed column: the booleans through `opt_bool`
/// (SQLite hands `BOOLEAN` back as an integer), `permlevel` through `opt_i64`
/// and the JSON array through `opt_str`. A `NULL` or absent column is the
/// rule default, which is the same meaning an empty field has in the Desk.
///
/// # Errors
///
/// A missing role, an out-of-range `permlevel` or malformed `custom_permissions`.
pub fn rule_from_row(row: &Row) -> Result<(PermissionRule, bool)> {
    let role = row.opt_str("role").unwrap_or_default().trim().to_string();
    if role.is_empty() {
        return Err(Error::Validation(ValidationError::Invalid(
            "a Custom DocPerm row needs a role".to_string(),
        )));
    }
    let flag = |name: &str| row.opt_bool(name).unwrap_or(false);
    let permlevel = row.opt_i64("permlevel").unwrap_or(0);
    let permlevel = u8::try_from(permlevel).map_err(|_| {
        Error::Validation(ValidationError::Invalid(format!(
            "Custom DocPerm permlevel {permlevel} is out of range"
        )))
    })?;
    let custom_permissions = match row.opt_str("custom_permissions").as_deref().map(str::trim) {
        Some(text) if !text.is_empty() => serde_json::from_str::<Vec<String>>(text)
            .map_err(|error| Error::Serialization(SerializationError::Json(error)))?,
        _ => Vec::new(),
    };
    Ok((
        PermissionRule {
            role,
            read: flag("read"),
            write: flag("write"),
            create: flag("create"),
            delete: flag("delete"),
            submit: flag("submit"),
            cancel: flag("cancel"),
            amend: flag("amend"),
            print: flag("print"),
            email: flag("email"),
            share: flag("share"),
            report: flag("report"),
            import: flag("import"),
            export: flag("export"),
            permlevel,
            if_owner: flag("if_owner"),
            apply_to_all_doctypes: false,
            custom_permissions,
        },
        flag("select"),
    ))
}
