//! Custom permission types (Frappe's `perm_type`) resolved from the effective
//! `Custom DocPerm` rules.

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
pub async fn has_custom_permission(
    pool: &ferris_db::Db,
    user: &ferris_core::User,
    doctype: &str,
    perm_type: &str,
) -> ferris_core::Result<bool> {
    ferris_schema::has_custom_permission(pool, user, doctype, perm_type).await
}

/// [`has_custom_permission`] for one document, including `if_owner` rules.
///
/// A DocType-level check cannot answer an `if_owner` grant (it needs the
/// stored owner), so the plain form deliberately skips those rules. A caller
/// that already loaded the document passes `owner` here and gets Frappe's full
/// semantics.
pub async fn has_custom_permission_on(
    pool: &ferris_db::Db,
    user: &ferris_core::User,
    doctype: &str,
    owner: Option<&str>,
    perm_type: &str,
) -> ferris_core::Result<bool> {
    ferris_schema::has_custom_permission_on(pool, user, doctype, owner, perm_type).await
}
