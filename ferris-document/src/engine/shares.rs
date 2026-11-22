//! Share-aware authorization for the engine (DOC8).
//!
//! `crate::sharing::ShareManager` shipped complete — `share`/`unshare`/
//! `get_shares`/`check_permission` — with **no production caller**. `tab_share`
//! was even cascaded on delete (`engine/write.rs`), so a grant could not outlive
//! a document, yet nothing ever wrote one. A shared document therefore did not
//! exist as a concept: the only ways to reach a document were the role rules in
//! `ferris_core::permissions` and the owner shortcut.
//!
//! This module is the seam. [`DocumentEngine::authorize`] is the engine's single
//! authorization entry point: it consults the share table first, then falls back
//! to the synchronous role/owner check. Keeping it in one place is deliberate —
//! a second, independently-updated permission path is exactly the divergence the
//! audit's `document_can` finding (DOC2) was about.
//!
//! Shares are read from the pool, not from the caller's transaction, because the
//! pool is what the REST store already authorizes against (`ferris-api`'s
//! `ensure_writable`). Granting a share and writing the document it grants in the
//! *same* transaction is not a flow the framework offers, so reading on the pool
//! cannot observe a half-committed grant.

use ferris_core::Result;
use ferris_core::Value;
use ferris_core::permissions::{
    Decision, OWNER_CONDITION, Permission, SYSTEM_MANAGER_ROLE, caller_principals,
    global_engine_ref,
};
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use crate::EngineContext;
use crate::engine::DocumentEngine;
use crate::sharing::{ShareManager, SharePermission, TAB_SHARE, share_table};

/// Side tables already created, keyed by `(Db::id, table)`.
///
/// The engine is rebuilt per request, so a per-instance cache would run the
/// `CREATE TABLE IF NOT EXISTS` DDL on every call. The identity is the database
/// handle's, not the engine's, which keeps a multi-database process (tests, one
/// pool per tenant) correct. A failed DDL is not recorded, so the next call
/// retries instead of hiding a broken schema.
static RUNTIME_TABLES: LazyLock<Mutex<HashSet<(u64, &'static str)>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn runtime_table_ready(db: &ferris_db::Db, table: &'static str) -> bool {
    RUNTIME_TABLES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(&(db.id(), table))
}

fn mark_runtime_table_ready(db: &ferris_db::Db, table: &'static str) {
    RUNTIME_TABLES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert((db.id(), table));
}

impl DocumentEngine {
    /// Ensure the runtime side tables exist, once per database (DOC7/DOC8).
    ///
    /// Both are created lazily like a doctype table — the same contract the
    /// engine already uses — so a site that ran no side-table migration still
    /// gets a working history and share table. The result is cached by
    /// [`ferris_db::Db::id`], not by engine instance: the API builds one engine
    /// per request, and a per-instance cache would put two DDL statements on
    /// every read.
    pub(super) async fn ensure_runtime_tables(&self) -> Result<()> {
        let db = self.db();
        if !runtime_table_ready(db, "tab_version") {
            crate::runtime::ensure_version_table(db).await?;
            mark_runtime_table_ready(db, "tab_version");
        }
        if !runtime_table_ready(db, TAB_SHARE) {
            db.ensure_table(TAB_SHARE, &share_table()).await?;
            db.ensure_index("idx_share_doc", TAB_SHARE, &["doctype", "docname"], false)
                .await?;
            mark_runtime_table_ready(db, TAB_SHARE);
        }
        Ok(())
    }

    /// Authorize `action` on `name`, consulting document shares (DOC8) and the
    /// caller's roles/ownership.
    ///
    /// The two are an **OR**, matching Frappe's `has_permission`, where a share
    /// is an additional grant rather than a replacement for the role rules: a
    /// user who is allowed by role stays allowed even with no share, and a user
    /// with a share is allowed even when the role rules alone would refuse.
    ///
    /// `trusted_owner` must come from stored state, never from a request body —
    /// the same rule as the underlying [`super::support::check_permission`].
    pub(super) async fn authorize(
        &self,
        ctx: &EngineContext<'_>,
        name: &str,
        trusted_owner: Option<&str>,
        action: Permission,
    ) -> Result<()> {
        match role_decision(ctx, name, trusted_owner, action) {
            // The rules already allow it; the common case, and no query runs.
            RoleDecision::Allow => Ok(()),
            // DOC-12: an explicit `Deny` is a revocation, and a share is an
            // additional grant, not an override. Falling through to the share
            // table here let one `tab_share` row defeat an operator's `forbid`.
            RoleDecision::Deny => {
                super::support::check_permission(ctx, name, trusted_owner, action)
            }
            RoleDecision::NoOpinion => {
                if let Some(permission) = share_permission_for(action)
                    && self.share_allows(ctx, name, permission).await?
                {
                    return Ok(());
                }
                // Neither path allowed it. Re-run the role check to produce its
                // typed error, so the message the caller sees is the one the
                // role rules would have produced (the share path adds nothing
                // when it denies).
                super::support::check_permission(ctx, name, trusted_owner, action)
            }
        }
    }

    /// Whether a stored share grants `permission` on `(ctx.doctype, name)` to the
    /// acting user.
    ///
    /// Returns `Ok(false)` when there is no acting user (a share is granted *to*
    /// someone) or when no share matches. A database error propagates — a
    /// permission check that cannot read the grants must fail loudly, not fall
    /// back to "denied" and hide a broken table.
    async fn share_allows(
        &self,
        ctx: &EngineContext<'_>,
        name: &str,
        permission: SharePermission,
    ) -> Result<bool> {
        let Some(user) = ctx.user.as_ref() else {
            return Ok(false);
        };
        self.share_manager()
            .check_permission(&ctx.doctype, name, &user.id, permission)
            .await
    }

    /// A [`ShareManager`] bound to this engine's database.
    fn share_manager(&self) -> ShareManager {
        ShareManager::with_db(self.db().clone())
    }

    /// The names, within `ctx.doctype`, that the acting user may read only
    /// because a share grants it (DOC8).
    ///
    /// Empty when there is no acting user or no share grants `Read`. `list`
    /// uses the set to admit those rows and to decide it cannot let SQL own the
    /// page — a share is not represented in the plan's role scope, so paging in
    /// SQL would silently drop shared documents.
    ///
    /// Reads every share for the doctype in one query and filters in memory:
    /// shares per document are few and this is once per list, not per row.
    pub(super) async fn readable_share_names(
        &self,
        ctx: &EngineContext<'_>,
    ) -> Result<std::collections::HashSet<String>> {
        let Some(user) = ctx.user.as_ref() else {
            return Ok(std::collections::HashSet::new());
        };
        let names = self
            .share_manager()
            .readable_docnames(&ctx.doctype, &user.id)
            .await?;
        // DOC-12: a share may not widen a row the role rules explicitly deny, so
        // a name whose decision is `Deny` is dropped here as well. The check is
        // in memory and the list is once per page, not once per row.
        Ok(names
            .into_iter()
            .filter(|docname| {
                role_decision(ctx, docname, None, Permission::Read) != RoleDecision::Deny
            })
            .collect())
    }
}

/// The share right that corresponds to a document action, if any.
///
/// A share grants read/write/share, not the lifecycle actions: `Submit` and
/// `Cancel` stay role-gated even on a shared document, because they move the
/// tracked truth of a transaction. `Amend` maps to write — it creates the
/// successor document, which is an edit. The read-like actions (`Print`,
/// `Email`, `Report`, `Export`) read the document and so need `Read`; `Import`
/// writes it.
const fn share_permission_for(action: Permission) -> Option<SharePermission> {
    match action {
        Permission::Read
        | Permission::Print
        | Permission::Email
        | Permission::Report
        | Permission::Export => Some(SharePermission::Read),
        Permission::Write | Permission::Create | Permission::Amend | Permission::Import => {
            Some(SharePermission::Write)
        }
        Permission::Share | Permission::Manage => Some(SharePermission::Share),
        Permission::Submit | Permission::Cancel | Permission::Delete => None,
    }
}

/// What the installed role/owner policy says about one action.
///
/// The distinction between [`RoleDecision::Deny`] and
/// [`RoleDecision::NoOpinion`] is the whole point (DOC-12): a boolean "not
/// allowed" folds an explicit revocation into the same answer as "no rule
/// applies", and those two must not behave alike when a share is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoleDecision {
    /// A statement (or the owner shortcut, or System Manager) granted it.
    Allow,
    /// A statement explicitly revoked it; deny wins over every share.
    Deny,
    /// No statement matched and the owner shortcut does not apply.
    NoOpinion,
}

/// Evaluate the role/owner rules alone, keeping `Deny` separate.
///
/// This is the tri-state form of [`super::support::check_permission`], with the
/// same inputs (stored owner, caller principals, the `is_owner` condition) so
/// the two can never disagree about who is allowed: `Allow` is exactly "the
/// predicate would return true", and `Deny` is exactly "it returned false
/// because the engine said so".
fn role_decision(
    ctx: &EngineContext<'_>,
    name: &str,
    trusted_owner: Option<&str>,
    action: Permission,
) -> RoleDecision {
    let Some(user) = ctx.user.as_ref() else {
        // The anonymous default-deny is not an operator revocation, but a share
        // is granted *to* someone, so it cannot rescue it either.
        return if ctx.allow_anonymous {
            RoleDecision::Allow
        } else {
            RoleDecision::Deny
        };
    };
    if user.has_role(SYSTEM_MANAGER_ROLE) {
        return RoleDecision::Allow;
    }
    let resource = ferris_core::permissions::Resource::new(&ctx.doctype, name)
        .with_owner(trusted_owner.unwrap_or(""));
    let is_owner = matches!(
        &resource.owner,
        Some(owner) if !owner.is_empty() && *owner == user.id
    );
    let context = HashMap::from([(OWNER_CONDITION.to_string(), Value::Bool(is_owner))]);
    let decision = {
        let engine = global_engine_ref()
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        engine.check_principals(&caller_principals(user), action, &resource, &context)
    };
    match decision {
        Decision::Allow => RoleDecision::Allow,
        Decision::Deny => RoleDecision::Deny,
        Decision::NotApplicable => {
            // The owner shortcut, exactly as in `ferris_core::permissions`: the
            // document's owner may read/write/amend it and gains nothing else.
            if matches!(
                action,
                Permission::Read | Permission::Write | Permission::Amend
            ) && is_owner
            {
                RoleDecision::Allow
            } else {
                RoleDecision::NoOpinion
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferris_core::permissions::{
        PermissionEngine, Policy, PolicyStatement, Principal, Resource, set_global_engine,
    };
    use ferris_core::{Result as CoreResult, Value};

    /// The engine is process-wide; serialise the tests that swap it.
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

    fn deny_scope(doctype: &str, role: &str) -> Restore {
        let serialised = SERIALISE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = global_engine_ref()
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut engine = PermissionEngine::default();
        engine.add_policy(Policy::new(vec![PolicyStatement::deny(
            vec![Principal::role(role)],
            vec![Permission::Read],
            // Both shapes a document check can use, so the deny reaches a read
            // whether it is evaluated per document or over the DocType.
            vec![Resource::new(doctype, "*"), Resource::new(doctype, "DOC-1")],
        )]));
        set_global_engine(engine);
        Restore {
            _serialised: serialised,
            previous,
        }
    }

    fn bob() -> ferris_core::User {
        let mut user =
            ferris_core::User::system_user("bob@example.com".to_string(), "Bob".to_string());
        user.id = "bob@example.com".to_string();
        user.roles = vec!["Reader".to_string()];
        user
    }

    /// DOC-12: a share is an additional grant, never a way around a `Deny`.
    #[tokio::test]
    async fn a_share_cannot_defeat_an_explicit_deny() -> CoreResult<()> {
        let doctype = "ShareDenyDoc";
        let _restore = deny_scope(doctype, "Reader");
        let db = ferris_db::memory_db().await?;
        crate::naming::NamingSeriesStore::ensure_table(&db).await?;
        let engine = DocumentEngine::new(db.clone());
        engine.ensure_runtime_tables().await?;

        let create_ctx = EngineContext::new(doctype, "SD-.#####").allow_anonymous(true);
        let created = engine
            .create(&create_ctx, ferris_core::value!({ "title": "denied" }))
            .await?;
        let name = created
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        // The share that would otherwise grant Bob the read.
        ShareManager::with_db(db)
            .share(
                doctype,
                &name,
                "bob@example.com",
                vec![SharePermission::Read],
                "ada@example.com",
            )
            .await?;

        let ctx = EngineContext::new(doctype, "SD-.#####").with_user(&bob());
        let err = engine
            .authorize(&ctx, &name, Some("ada@example.com"), Permission::Read)
            .await
            .expect_err("a share must not defeat an explicit deny");
        assert!(matches!(err, ferris_core::Error::Authz(_)), "{err:?}");

        assert!(
            engine.readable_share_names(&ctx).await?.is_empty(),
            "a denied row must not be readmitted by its share in a list"
        );
        Ok(())
    }

    /// The tri-state keeps the ordinary share grant working: no statement
    /// matches, the share does.
    #[tokio::test]
    async fn a_share_still_grants_when_no_statement_applies() -> CoreResult<()> {
        let doctype = "ShareAllowDoc";
        // A struct, not a bare guard: holding the guard in a local across the
        // awaits below would be the exact lint (`await_holding_lock`) this
        // shape avoids while still serialising the engine swap.
        let _restore = {
            let serialised = SERIALISE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let previous = global_engine_ref()
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            set_global_engine(PermissionEngine::default());
            Restore {
                _serialised: serialised,
                previous,
            }
        };

        let db = ferris_db::memory_db().await?;
        crate::naming::NamingSeriesStore::ensure_table(&db).await?;
        let engine = DocumentEngine::new(db.clone());
        engine.ensure_runtime_tables().await?;
        let create_ctx = EngineContext::new(doctype, "SA-.#####").allow_anonymous(true);
        let created = engine
            .create(&create_ctx, ferris_core::value!({ "title": "shared" }))
            .await?;
        let name = created
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        ShareManager::with_db(db)
            .share(
                doctype,
                &name,
                "bob@example.com",
                vec![SharePermission::Read],
                "ada@example.com",
            )
            .await?;
        let ctx = EngineContext::new(doctype, "SA-.#####").with_user(&bob());
        engine
            .authorize(&ctx, &name, Some("ada@example.com"), Permission::Read)
            .await?;
        assert!(engine.readable_share_names(&ctx).await?.contains(&name));
        Ok(())
    }
}
