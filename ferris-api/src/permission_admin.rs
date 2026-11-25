//! The Role Permissions Manager's server half: custom permission types.
//!
//! Frappe's Permission Types
//! (<https://docs.frappe.io/framework/permission-types>) are declared per
//! DocType and then assigned to roles in the Role Permission Manager. Ferris
//! stores the assignment on the site-specific `Custom DocPerm` row
//! (`custom_permissions`), because that row is already the override layer the
//! metadata and the runtime policy both read.
//!
//! The matrix endpoints are deliberately **System Manager only**: they expose
//! every role's rights, and a write changes what every user of that role may do.
//! A write rebuilds the installed policy through the same path a boot uses, so
//! a checkbox takes effect immediately instead of at the next restart.

use std::collections::HashSet;

use ferris_core::permissions::{PermissionRule, STANDARD_ROLES, SYSTEM_MANAGER_ROLE};
use ferris_core::{Error, Result, SerializationError, ValidationError, Value};
use ferris_db::sql::{Column, Ident};
use ferris_db::{Db, Stmt};
use ferris_schema::contract::{CustomPermissionGrant, PermissionMatrix};

use crate::custom_docperm::{effective_permissions, for_doctype, rule_from_row};
use crate::store::DocStore;

const CUSTOM_DOC_PERM_TABLE: &str = "tabCustomDocPerm";

fn require_admin(user: &ferris_core::User, doctype: &str) -> Result<()> {
    if user.has_role(SYSTEM_MANAGER_ROLE) {
        return Ok(());
    }
    Err(Error::Authz(ferris_core::AuthzError::PermissionDenied {
        action: "manage_permissions".to_string(),
        resource: doctype.to_string(),
    }))
}

/// The full role × permission-type matrix for one DocType.
pub async fn matrix(
    pool: &Db,
    user: &ferris_core::User,
    doctype: &str,
) -> Result<PermissionMatrix> {
    require_admin(user, doctype)?;
    if ferris_schema::get_doctype(doctype).is_none() {
        return Err(Error::Schema(ferris_core::SchemaError::DocTypeNotFound(
            doctype.to_string(),
        )));
    }
    let perm_types: Vec<String> = crate::doc_meta::permission_types(pool, doctype)
        .await?
        .into_iter()
        .map(|permission| permission.perm_type)
        .collect();
    let roles = roles(pool).await?;
    let granted: HashSet<(String, String)> = for_doctype(pool, doctype)
        .await?
        .into_iter()
        .flat_map(|(rule, _)| {
            let role = rule.role;
            rule.custom_permissions
                .into_iter()
                .map(move |perm_type| (perm_type, role.clone()))
        })
        .collect();
    let grants = perm_types
        .iter()
        .flat_map(|perm_type| {
            roles.iter().map(|role| CustomPermissionGrant {
                perm_type: perm_type.clone(),
                role: role.clone(),
                granted: granted.contains(&(perm_type.clone(), role.clone())),
            })
        })
        .collect();
    Ok(PermissionMatrix {
        doctype: doctype.to_string(),
        roles,
        perm_types,
        grants,
        editable: true,
    })
}

/// Grant or revoke one permission type for one role.
///
/// Revoking also needs no synthetic row: the type is removed from the existing
/// `custom_permissions` array and the row stays, preserving any other override
/// the role already carries.
pub async fn set_grant(
    pool: &Db,
    user: &ferris_core::User,
    doctype: &str,
    role: &str,
    perm_type: &str,
    granted: bool,
) -> Result<()> {
    require_admin(user, doctype)?;
    let role = role.trim();
    let perm_type = perm_type.trim();
    if role.is_empty() || perm_type.is_empty() {
        return Err(Error::Validation(ValidationError::Invalid(
            "a custom permission grant needs a role and a permission type".to_string(),
        )));
    }
    let declared = crate::doc_meta::permission_types(pool, doctype).await?;
    if !declared
        .iter()
        .any(|permission| permission.perm_type == perm_type)
    {
        return Err(Error::Validation(ValidationError::Invalid(format!(
            "`{perm_type}` is not a Permission Type declared for `{doctype}`"
        ))));
    }
    let Some(rules) = effective_permissions(pool, doctype).await? else {
        return Err(Error::Schema(ferris_core::SchemaError::DocTypeNotFound(
            doctype.to_string(),
        )));
    };

    if let Some((row_name, mut rule)) = find_custom_row(pool, doctype, role).await? {
        if granted {
            if !rule
                .custom_permissions
                .iter()
                .any(|value| value == perm_type)
            {
                rule.custom_permissions.push(perm_type.to_string());
            }
        } else {
            rule.custom_permissions.retain(|value| value != perm_type);
        }
        persist_row(pool, doctype, &row_name, &rule.custom_permissions).await?;
        return Ok(());
    }

    if !granted {
        return Ok(());
    }
    // The first grant for a role clones that role's current rule, so an
    // override never silently drops the read/write/... rights it did not mean
    // to touch. `select` stays 0: the row replaces only the same permlevel.
    let mut rule = rules
        .iter()
        .find(|existing| existing.role == role && existing.permlevel == 0)
        .cloned()
        .unwrap_or_else(|| PermissionRule {
            role: role.to_string(),
            ..PermissionRule::default()
        });
    rule.role = role.to_string();
    rule.custom_permissions = vec![perm_type.to_string()];
    let created = DocStore::create(pool, "Custom DocPerm", rule_payload(&rule, doctype)?).await?;
    let name = created
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if name.is_empty() {
        return Err(Error::Validation(ValidationError::Invalid(
            "the created Custom DocPerm row has no name".to_string(),
        )));
    }
    set_target(pool, name, doctype).await?;
    Ok(())
}

/// Pin a `Custom DocPerm` row to the DocType it overrides.
///
/// `parent` is the row's real link to its DocType, but the document engine
/// treats it as a structural field and leaves it out of a non-child table's
/// stored columns, so a row created through [`DocStore`] does not carry it.
/// The declared column exists on a migrated site; `ensure_column` covers a
/// table the engine created lazily before the migration ran, and tolerates the
/// column already being there.
async fn set_target(pool: &Db, name: &str, doctype: &str) -> Result<()> {
    let quoted = Ident::new(CUSTOM_DOC_PERM_TABLE)?.quote(pool.dialect());
    pool.ensure_column(
        CUSTOM_DOC_PERM_TABLE,
        &Column::new("parent", "VARCHAR(255)"),
    )
    .await?;
    pool.execute(&Stmt::new(
        format!("UPDATE {quoted} SET parent = ? WHERE name = ?"),
        vec![Value::from(doctype), Value::from(name)],
    ))
    .await?;
    Ok(())
}

/// The roles the matrix offers: the `Role` DocType's rows plus the framework's
/// standard roles, so a seed that never created the table still yields a usable
/// matrix instead of an empty one.
async fn roles(pool: &Db) -> Result<Vec<String>> {
    const ROLE_TABLE: &str = "tabRole";
    let mut roles: Vec<String> = Vec::new();
    if pool.table_exists(ROLE_TABLE).await? {
        let quoted = Ident::new(ROLE_TABLE)?.quote(pool.dialect());
        let rows = pool
            .fetch_all(&Stmt::new(
                format!("SELECT name FROM {quoted} ORDER BY name"),
                Vec::new(),
            ))
            .await?;
        for row in rows {
            let name = row.str("name")?;
            if !name.trim().is_empty() {
                roles.push(name);
            }
        }
    }
    for standard in STANDARD_ROLES {
        if !roles.iter().any(|role| role == standard) {
            roles.push((*standard).to_string());
        }
    }
    roles.sort();
    roles.dedup();
    Ok(roles)
}

/// The stored `Custom DocPerm` row for `(doctype, role, permlevel 0)`, if any.
///
/// Rows are read as their real columns, never from a `data` blob (ADR 0001):
/// the selection and the rule decoding both go through the column names, so a
/// `SELECT *` is enough and a partial schema reads the missing right columns as
/// their defaults. `parent` and `role` are the indexed lookup keys, and
/// `COALESCE(permlevel, 0) = 0` keeps a `NULL` level meaning level 0 exactly as
/// [`rule_from_row`] decodes it.
async fn find_custom_row(
    pool: &Db,
    doctype: &str,
    role: &str,
) -> Result<Option<(String, PermissionRule)>> {
    if !pool.table_exists(CUSTOM_DOC_PERM_TABLE).await? {
        return Ok(None);
    }
    let quoted = Ident::new(CUSTOM_DOC_PERM_TABLE)?.quote(pool.dialect());
    let row = pool
        .fetch_optional(&Stmt::new(
            format!(
                "SELECT * FROM {quoted} \
                 WHERE parent = ? AND role = ? AND COALESCE(permlevel, 0) = 0 \
                 ORDER BY name LIMIT 1"
            ),
            vec![Value::from(doctype), Value::from(role)],
        ))
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let (rule, _) = rule_from_row(&row)?;
    Ok(Some((row.str("name")?, rule)))
}

/// Rewrite only the `custom_permissions` column of a stored row.
///
/// The rest of the row (the role's rights, `select`, `permlevel`) is left
/// untouched, so granting a permission type cannot silently drop a right. The
/// effective metadata embeds `Custom DocPerm`, so a write bumps the generation
/// exactly like the generic `DocStore` path does.
async fn persist_row(
    pool: &Db,
    doctype: &str,
    name: &str,
    custom_permissions: &[String],
) -> Result<()> {
    let quoted = Ident::new(CUSTOM_DOC_PERM_TABLE)?.quote(pool.dialect());
    pool.execute(&Stmt::new(
        format!("UPDATE {quoted} SET custom_permissions = ?, modified = ? WHERE name = ?"),
        vec![
            Value::from(serialized_list(custom_permissions)?),
            Value::from(chrono::Utc::now().timestamp()),
            Value::from(name),
        ],
    ))
    .await?;
    // The effective metadata embeds `Custom DocPerm`; a write must bump the
    // per-DocType generation or the cached copy keeps serving the old policy
    // (PERF-03/SCH-04).
    ferris_schema::customization::CustomizationStore::invalidate_meta(doctype);
    Ok(())
}

fn serialized_list(values: &[String]) -> Result<String> {
    serde_json::to_string(values)
        .map_err(|error| Error::Serialization(SerializationError::Json(error)))
}

/// The document body that creates a `Custom DocPerm` row from a rule.
fn rule_payload(rule: &PermissionRule, doctype: &str) -> Result<Value> {
    Ok(Value::from(serde_json::json!({
        "parent": doctype,
        "role": rule.role,
        "permlevel": rule.permlevel,
        "read": rule.read,
        "write": rule.write,
        "create": rule.create,
        "delete": rule.delete,
        "submit": rule.submit,
        "cancel": rule.cancel,
        "amend": rule.amend,
        "print": rule.print,
        "email": rule.email,
        "share": rule.share,
        "report": rule.report,
        "import": rule.import,
        "export": rule.export,
        "if_owner": rule.if_owner,
        "select": false,
        "custom_permissions": serialized_list(&rule.custom_permissions)?,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferris_schema::{DoctypeMeta, FieldMeta, RolePermission, register_doctype};

    fn admin() -> ferris_core::User {
        let mut user =
            ferris_core::User::system_user("admin@local".to_string(), "Admin".to_string());
        user.id = "admin@local".to_string();
        user.roles = vec![SYSTEM_MANAGER_ROLE.to_string()];
        user
    }

    fn granted(matrix: &PermissionMatrix, role: &str, perm_type: &str) -> bool {
        matrix
            .grants
            .iter()
            .find(|grant| grant.role == role && grant.perm_type == perm_type)
            .is_some_and(|grant| grant.granted)
    }

    /// The matrix is stored on `Custom DocPerm.custom_permissions`; a grant and
    /// a revoke must round-trip through the same rows the runtime policy reads.
    #[tokio::test]
    async fn granting_and_revoking_a_permission_type_round_trips() {
        let pool = ferris_db::memory_db().await.unwrap();
        ferris_schema::doctypes::register_doctypes();
        crate::doctypes::register_doctypes();
        register_doctype(
            DoctypeMeta::new("PermProbe", "Test")
                .field(FieldMeta::data("title"))
                .permission(
                    RolePermission::new(SYSTEM_MANAGER_ROLE)
                        .read()
                        .write()
                        .create()
                        .delete(),
                ),
        );
        let admin = admin();
        ferris_document::with_user(Some(admin.clone()), async {
            DocStore::create(
                &pool,
                "Permission Type",
                serde_json::json!({
                    "name": "PermProbe-approve",
                    "document_type": "PermProbe",
                    "perm_type": "approve",
                })
                .into(),
            )
            .await
            .expect("the admin declares a permission type");

            let matrix_value = matrix(&pool, &admin, "PermProbe").await.unwrap();
            assert!(
                matrix_value.perm_types.contains(&"approve".to_string()),
                "the declared type must reach the matrix"
            );
            assert!(!granted(&matrix_value, SYSTEM_MANAGER_ROLE, "approve"));

            set_grant(
                &pool,
                &admin,
                "PermProbe",
                SYSTEM_MANAGER_ROLE,
                "approve",
                true,
            )
            .await
            .unwrap();
            let matrix_value = matrix(&pool, &admin, "PermProbe").await.unwrap();
            assert!(granted(&matrix_value, SYSTEM_MANAGER_ROLE, "approve"));

            // The metadata path the Desk and business code read must resolve
            // the same grant for a non-admin role.
            let mut approver = ferris_core::User::system_user(
                "approver@local".to_string(),
                "Approver".to_string(),
            );
            approver.id = "approver@local".to_string();
            approver.roles = vec!["Approver".to_string()];
            assert!(
                !crate::authz::has_custom_permission(&pool, &approver, "PermProbe", "approve")
                    .await
                    .unwrap()
            );
            set_grant(&pool, &admin, "PermProbe", "Approver", "approve", true)
                .await
                .unwrap();
            assert!(
                crate::authz::has_custom_permission(&pool, &approver, "PermProbe", "approve")
                    .await
                    .unwrap()
            );

            set_grant(
                &pool,
                &admin,
                "PermProbe",
                SYSTEM_MANAGER_ROLE,
                "approve",
                false,
            )
            .await
            .unwrap();
            let matrix_value = matrix(&pool, &admin, "PermProbe").await.unwrap();
            assert!(!granted(&matrix_value, SYSTEM_MANAGER_ROLE, "approve"));
        })
        .await;
    }

    /// A non-System-Manager must not read the matrix: it exposes every role's
    /// rights, and the refusal is a typed `PermissionDenied`.
    #[tokio::test]
    async fn the_matrix_is_system_manager_only() {
        let pool = ferris_db::memory_db().await.unwrap();
        let mut user =
            ferris_core::User::system_user("alice@local".to_string(), "Alice".to_string());
        user.id = "alice@local".to_string();
        let error = matrix(&pool, &user, "PermProbe").await.unwrap_err();
        assert!(matches!(
            error,
            Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
        ));
    }
}
