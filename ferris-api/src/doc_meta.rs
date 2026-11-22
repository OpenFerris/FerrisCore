//! DocType Actions, Links (Connections) and Permission Types.
//!
//! Frappe keeps these three as ordinary DocTypes (`DocType Action`,
//! `DocType Link`, `Permission Type`) and folds them into the metadata payload:
//!
//! * Actions create buttons on a form
//!   (<https://docs.frappe.io/framework/user/en/basics/doctypes/actions-and-links>).
//! * Links describe the **Connections** panel, including links that live inside
//!   a child table (`table_fieldname`).
//! * Permission Types declare action-specific rights such as `approve`
//!   (<https://docs.frappe.io/framework/permission-types>).
//!
//! The rows are site data, so they are read from storage here instead of being
//! compiled into the registry. `effective_meta` keeps owning the merge, which is
//! why a permission row and the Desk payload can never disagree.

use ferris_core::{Result, Value};
use ferris_db::Db;
use ferris_schema::contract::{DynDocLink, DynMeta};
use serde::Serialize;
use std::collections::HashMap;

pub use ferris_schema::contract::{ConnectionDocument, ConnectionGroup, Connections};

use crate::store::{DocStore, effective_meta, validate_doctype, validate_name};
use crate::types::ListParams;

/// The complete `GET /api/doctype/meta` payload: the effective registry metadata
/// plus the stored Actions, Links and Permission Types.
///
/// Both the REST handler and the SSR bridge call this one function, so the
/// hydrated and the server-rendered Desk receive byte-identical metadata. The
/// projection itself lives in `ferris_schema::registry::dyn_meta`, which the
/// WASM `data` import also uses.
pub async fn effective_dyn_meta(pool: &Db, doctype: &str) -> Result<Option<DynMeta>> {
    let Some(meta) = effective_meta(pool, doctype).await? else {
        return Ok(None);
    };
    ferris_schema::registry::dyn_meta(pool, &meta)
        .await
        .map(Some)
}

pub use ferris_schema::registry::{doc_actions, doc_links, permission_types};

/// Resolve every `DocType Link` of `doctype` against the document `name`.
///
/// External links are a `WHERE link_fieldname = ?` list, which the engine
/// filters by the caller's `Read` permission. Internal links live inside a child
/// table, so there is no column to filter: the parent rows are paged (hard cap
/// [`CONNECTION_SCAN_LIMIT`]) and the array is scanned in Rust. That is explicit
/// rather than pretending the JSON blob is indexed.
pub async fn connections(pool: &Db, doctype: &str, name: &str) -> Result<Connections> {
    validate_doctype(doctype)?;
    validate_name(name)?;
    let mut groups = Vec::new();
    for link in doc_links(pool, doctype).await? {
        if link.hidden || link.link_doctype.is_empty() || link.link_fieldname.is_empty() {
            continue;
        }
        let title_field = effective_meta(pool, &link.link_doctype)
            .await?
            .and_then(|meta| meta.title_field);
        let (documents, count) = match link.table_fieldname.as_deref() {
            Some(table) if !table.is_empty() => {
                scan_internal_links(pool, &link, name, table).await?
            }
            _ => {
                let params = ListParams {
                    filters: Some(HashMap::from([(
                        link.link_fieldname.clone(),
                        Value::from(name),
                    )])),
                    limit: Some(CONNECTION_SCAN_LIMIT),
                    ..ListParams::default()
                };
                let response = DocStore::list(pool, &link.link_doctype, &params).await?;
                let documents = response
                    .data
                    .into_iter()
                    .map(|document| connection_document(document, title_field.as_deref()))
                    .collect();
                (documents, response.total)
            }
        };
        if documents.is_empty() {
            continue;
        }
        groups.push(ConnectionGroup {
            link_doctype: link.link_doctype,
            link_fieldname: link.link_fieldname,
            group: link.group,
            count,
            documents,
        });
    }
    Ok(Connections {
        connections: groups,
    })
}

/// Hard cap on rows scanned for an internal (child-table) connection.
const CONNECTION_SCAN_LIMIT: u32 = 100;

async fn scan_internal_links(
    pool: &Db,
    link: &DynDocLink,
    name: &str,
    table: &str,
) -> Result<(Vec<ConnectionDocument>, u64)> {
    let params = ListParams {
        limit: Some(CONNECTION_SCAN_LIMIT),
        ..ListParams::default()
    };
    let response = DocStore::list(pool, &link.link_doctype, &params).await?;
    let mut documents = Vec::new();
    for document in response.data {
        let matched = document
            .get(table)
            .and_then(Value::as_array)
            .is_some_and(|rows| {
                rows.iter()
                    .any(|row| row.get(&link.link_fieldname).and_then(Value::as_str) == Some(name))
            });
        if matched {
            documents.push(connection_document(document, None));
        }
    }
    let count = documents.len() as u64;
    Ok((documents, count))
}

fn connection_document(document: Value, title_field: Option<&str>) -> ConnectionDocument {
    let name = document
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let title = title_field
        .and_then(|field| document.get(field).and_then(Value::as_str))
        .filter(|title| !title.is_empty())
        .map(str::to_string);
    ConnectionDocument { name, title }
}

/// One declared custom permission and whether the caller holds it.
#[derive(Debug, Clone, Serialize)]
pub struct CustomPermissionStatus {
    pub perm_type: String,
    pub granted: bool,
}

/// The declared permission types of `doctype`, resolved for `user`.
pub async fn permission_inventory(
    pool: &Db,
    user: &ferris_core::User,
    doctype: &str,
) -> Result<Vec<CustomPermissionStatus>> {
    let declared = permission_types(pool, doctype).await?;
    let mut statuses = Vec::with_capacity(declared.len());
    for permission in declared {
        let granted =
            crate::authz::has_custom_permission(pool, user, doctype, &permission.perm_type).await?;
        statuses.push(CustomPermissionStatus {
            perm_type: permission.perm_type,
            granted,
        });
    }
    Ok(statuses)
}

/// One installed `User Permission` rule, as the Desk sees it.
#[derive(Debug, Clone, Serialize)]
pub struct UserPermissionEntry {
    /// The `DocType` whose values are allowed.
    pub allow: String,
    /// The one allowed value of [`Self::allow`].
    pub for_value: String,
    /// The `DocType` narrowed when the rule is scoped to one, if any.
    pub applicable_for: Option<String>,
    /// Whether the rule reaches every `DocType` linking to [`Self::allow`].
    pub apply_to_all_doctypes: bool,
    /// Whether the descendants of a tree [`Self::for_value`] are included.
    pub hide_descendants: bool,
}

/// The `User Permission` rules installed for `user`, optionally for one allowed
/// `DocType`.
///
/// This is the **visible** half of the restriction: the Desk uses it to label a
/// picker ("Customer A only") and the API to report what is in force. The filter
/// itself is **not** re-implemented here — it is applied by the document engine
/// on every read (`read_scope` + the per-row check) and on every write, so the
/// options a `Link` picker receives through `GET /api/resource/<DocType>` are
/// already narrowed. Filtering again in the client would be a second policy
/// that could drift from the enforced one.
///
/// # Errors
///
/// Database or document-decoding errors from the store read.
pub async fn user_permissions(
    pool: &Db,
    user: &ferris_core::User,
    allow: Option<&str>,
) -> Result<Vec<UserPermissionEntry>> {
    let rules = ferris_auth::user_permission::UserPermissionStore::new(pool.clone())
        .list_for_user(&user.id)
        .await?;
    Ok(rules
        .into_iter()
        .filter(|rule| allow.is_none_or(|wanted| rule.allow == wanted))
        .map(|rule| UserPermissionEntry {
            allow: rule.allow,
            for_value: rule.for_value,
            applicable_for: rule.applicable_for,
            apply_to_all_doctypes: rule.apply_to_all_doctypes,
            hide_descendants: rule.hide_descendants,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admin() -> ferris_core::User {
        let mut user =
            ferris_core::User::system_user("admin@local".to_string(), "Admin".to_string());
        user.id = "admin@local".to_string();
        user.roles = vec![ferris_core::permissions::SYSTEM_MANAGER_ROLE.to_string()];
        user
    }

    /// FRA-03: the payload reports the rules the engine enforces, optionally
    /// narrowed to one allowed `DocType`, so the Desk can label a picker without
    /// re-implementing the filter.
    #[tokio::test]
    async fn user_permissions_are_reported_for_the_desk() {
        let pool = ferris_db::memory_db().await.unwrap();
        let store = ferris_auth::user_permission::UserPermissionStore::new(pool.clone());
        store.init().await.unwrap();
        store
            .upsert(
                &ferris_core::permissions::UserPermissionRule::new(
                    "ada@local",
                    "Customer",
                    "CUST-A",
                )
                .with_applicable_for("Sales Order"),
            )
            .await
            .unwrap();

        let mut user = ferris_core::User::system_user("ada@local".to_string(), "Ada".to_string());
        user.id = "ada@local".to_string();
        let entries = user_permissions(&pool, &user, None).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].allow, "Customer");
        assert_eq!(entries[0].for_value, "CUST-A");
        assert_eq!(entries[0].applicable_for.as_deref(), Some("Sales Order"));
        assert!(
            user_permissions(&pool, &user, Some("Territory"))
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// The stored rows live in the engine's physical tables (`tabDocTypeAction`,
    /// not `tabDocType Action`). This pins that name: using the display name
    /// silently returned an empty list, which is exactly the kind of "metadata
    /// that never loads" a whitespace difference can hide.
    #[tokio::test]
    async fn stored_actions_links_and_permission_types_are_read() {
        let pool = ferris_db::memory_db().await.unwrap();
        ferris_schema::doctypes::register_doctypes();
        crate::doctypes::register_doctypes();
        let admin = admin();
        ferris_document::with_user(Some(admin), async {
            crate::store::DocStore::create(
                &pool,
                "DocType Action",
                serde_json::json!({
                    "name": "PermProbe-approve-action",
                    "document_type": "PermProbe",
                    "action_type": "Route",
                    "action": "/desk/Test/PermProbe/1",
                    "label": "Aprobar",
                })
                .into(),
            )
            .await
            .unwrap();
            crate::store::DocStore::create(
                &pool,
                "DocType Link",
                serde_json::json!({
                    "name": "PermProbe-note-link",
                    "document_type": "PermProbe",
                    "link_doctype": "ToDo",
                    "link_fieldname": "reference_name",
                })
                .into(),
            )
            .await
            .unwrap();
            crate::store::DocStore::create(
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
            .unwrap();

            let actions = doc_actions(&pool, "PermProbe").await.unwrap();
            assert_eq!(actions.len(), 1);
            assert_eq!(actions[0].name, "PermProbe-approve-action");
            assert_eq!(actions[0].label, "Aprobar");

            let links = doc_links(&pool, "PermProbe").await.unwrap();
            assert_eq!(links.len(), 1);
            assert_eq!(links[0].name, "PermProbe-note-link");
            assert_eq!(links[0].link_fieldname, "reference_name");

            let permissions = permission_types(&pool, "PermProbe").await.unwrap();
            assert_eq!(permissions.len(), 1);
            assert_eq!(permissions[0].perm_type, "approve");
        })
        .await;
    }
}
