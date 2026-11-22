//! Storage, cache and installation for `User Permission` (FRA-03).
//!
//! The `DocType` and the Desk screen existed with **no reader**: an operator could
//! restrict a user to one `Customer` and nothing enforced it. `ferris-core`
//! owns the rule shape and the synchronous port the document engine calls
//! ([`ferris_core::permissions::UserPermissionSource`]); this module is the
//! database-backed half — the store that reads and writes the rows, the
//! process-wide snapshot the port serves, and the install that wires the two
//! together at boot.
//!
//! # One query per refresh, none per row
//!
//! The engine asks the port for a user's rules on every row it authorizes, so
//! the port must not touch the database. [`refresh`] loads every row in one
//! engine `list`, expands the tree rules, and replaces the snapshot; the port
//! then answers from memory. The snapshot is refreshed when a `User Permission`
//! row is written or deleted (the hooks [`register_hooks`] installs) and at
//! boot, so an operator's edit is live on the next query without putting a
//! query on the request path.
//!
//! # `hide_descendants`
//!
//! The flag is only meaningful when the allowed `DocType` is a tree. The tree
//! walk needs the document table, so it happens here, once per refresh: the
//! rule carries the resolved descendant values and the per-row check stays a
//! string comparison. A rule that sets the flag on a non-tree (or unknown)
//! `DocType` is **rejected**, both when the row is validated (a `BeforeValidate`
//! hook, so the operator sees a `400`) and again when the snapshot is loaded
//! (defence in depth), instead of being silently ignored.

use ferris_core::hooks::{HookPoint, HookRegistry};
use ferris_core::permissions::{
    UserPermissionRule, UserPermissionRules, UserPermissionSource, set_user_permission_source,
};
use ferris_core::{Error, Result, ValidationError, Value};
use ferris_db::Db;
use ferris_document::{DocumentEngine, ListQuery};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use crate::store_support::system_context;

/// The `DocType` this module stores.
pub const DOCTYPE: &str = "User Permission";

/// Hook tag, so a re-install (a hot reload, a second boot in a test) replaces
/// the hooks instead of stacking another copy of them.
const HOOK_REFRESH: &str = "user-permission-cache";
const HOOK_VALIDATE: &str = "user-permission-validate";

/// The field the tree walk uses when a `DocType` does not name its parent field.
const DEFAULT_TREE_PARENT: &str = "parent";

/// The user's rules, keyed by user id, shared so a lookup clones an `Arc`.
static SNAPSHOT: LazyLock<RwLock<HashMap<String, UserPermissionRules>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// The empty set, for users no rule mentions.
static NO_RULES: LazyLock<UserPermissionRules> =
    LazyLock::new(|| UserPermissionRules::from(Vec::new()));

/// A `User Permission` row together with the document name it is stored under.
///
/// The name is the update/delete key; the rule itself is the policy shape the
/// engine consumes.
#[derive(Debug, Clone)]
struct StoredRule {
    name: String,
    rule: UserPermissionRule,
}

impl StoredRule {
    fn from_document(document: &Value) -> Self {
        let text = |field: &str| {
            document
                .get(field)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let flag = |field: &str| {
            document
                .get(field)
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        let applicable_for = text("applicable_for");
        Self {
            name: text("name"),
            rule: UserPermissionRule {
                user: text("user"),
                allow: text("allow"),
                for_value: text("for_value"),
                applicable_for: (!applicable_for.is_empty()).then_some(applicable_for),
                apply_to_all_doctypes: flag("apply_to_all_doctypes"),
                hide_descendants: flag("hide_descendants"),
                descendants: Vec::new(),
            },
        }
    }
}

/// Persisted `User Permission` storage.
///
/// Reads and writes go through the document engine as `System Manager`
/// (`store_support::system_context`), like `DocPermStore`: the framework's own
/// tables are documents too, so validation, hooks and field permissions run on
/// one code path. Only a `System Manager` may write the rows themselves (their
/// declaration grants nothing else), which is what keeps a user from widening
/// their own restriction.
#[derive(Debug, Clone)]
pub struct UserPermissionStore {
    engine: DocumentEngine,
}

impl UserPermissionStore {
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self {
            engine: DocumentEngine::new(db),
        }
    }

    /// Register the declaration and ensure the table exists. Idempotent.
    ///
    /// # Errors
    ///
    /// Database errors from creating the table.
    pub async fn init(&self) -> Result<()> {
        ferris_schema::register_doctype(crate::doctypes::user_permission_meta());
        self.engine.ensure_table(DOCTYPE).await.map(|_| ())
    }

    /// Every stored rule, in key order.
    ///
    /// # Errors
    ///
    /// Database or document-decoding errors.
    pub async fn list_all(&self) -> Result<Vec<UserPermissionRule>> {
        let mut rows = self.fetch().await?;
        rows.sort_by(|a, b| {
            (&a.rule.user, &a.rule.allow, &a.rule.for_value).cmp(&(
                &b.rule.user,
                &b.rule.allow,
                &b.rule.for_value,
            ))
        });
        Ok(rows.into_iter().map(|stored| stored.rule).collect())
    }

    /// The rules of one user.
    ///
    /// # Errors
    ///
    /// Database or document-decoding errors.
    pub async fn list_for_user(&self, user: &str) -> Result<Vec<UserPermissionRule>> {
        Ok(self
            .list_all()
            .await?
            .into_iter()
            .filter(|rule| rule.user == user)
            .collect())
    }

    /// Insert or overwrite the row for `(user, allow, for_value)`.
    ///
    /// The natural key is Frappe's: a user has at most one rule per allowed
    /// value of a `DocType`, so re-saving updates instead of duplicating.
    ///
    /// # Errors
    ///
    /// Database errors and the validation the engine runs on the document.
    pub async fn upsert(&self, rule: &UserPermissionRule) -> Result<()> {
        let fields = rule_fields(rule);
        let existing = self
            .fetch()
            .await?
            .into_iter()
            .find(|stored| same_key(&stored.rule, rule));
        let meta = crate::doctypes::user_permission_meta();
        let ctx = system_context(&meta);
        match existing {
            Some(stored) => self
                .engine
                .update(&ctx, &stored.name, fields)
                .await
                .map(|_| ()),
            None => self.engine.create(&ctx, fields).await.map(|_| ()),
        }
    }

    /// Remove the row for `(user, allow, for_value)`.
    ///
    /// # Errors
    ///
    /// Database errors from the delete.
    pub async fn delete(&self, user: &str, allow: &str, for_value: &str) -> Result<()> {
        let wanted = UserPermissionRule::new(user, allow, for_value);
        let meta = crate::doctypes::user_permission_meta();
        let ctx = system_context(&meta);
        for stored in self
            .fetch()
            .await?
            .into_iter()
            .filter(|stored| same_key(&stored.rule, &wanted))
        {
            self.engine.delete(&ctx, &stored.name).await?;
        }
        Ok(())
    }

    /// The engine `list` call: the table is small, so every read asks for all
    /// rows and the row-by-row filtering buys nothing.
    async fn fetch(&self) -> Result<Vec<StoredRule>> {
        let meta = crate::doctypes::user_permission_meta();
        let ctx = system_context(&meta);
        let query = ListQuery {
            limit: Some(u32::MAX),
            ..ListQuery::default()
        };
        let (documents, _) = self.engine.list(&ctx, &query).await?;
        Ok(documents.iter().map(StoredRule::from_document).collect())
    }
}

/// The document fields of one rule.
fn rule_fields(rule: &UserPermissionRule) -> Value {
    let mut map = ferris_core::Map::new();
    map.insert("user".to_string(), Value::from(rule.user.clone()));
    map.insert("allow".to_string(), Value::from(rule.allow.clone()));
    map.insert("for_value".to_string(), Value::from(rule.for_value.clone()));
    map.insert(
        "applicable_for".to_string(),
        rule.applicable_for.clone().map_or(Value::Null, Value::from),
    );
    map.insert(
        "apply_to_all_doctypes".to_string(),
        Value::from(rule.apply_to_all_doctypes),
    );
    map.insert(
        "hide_descendants".to_string(),
        Value::from(rule.hide_descendants),
    );
    Value::Object(map)
}

/// Whether two rules address the same stored row.
fn same_key(a: &UserPermissionRule, b: &UserPermissionRule) -> bool {
    a.user == b.user && a.allow == b.allow && a.for_value == b.for_value
}

/// The rules installed for `user`, served from the snapshot.
///
/// This is what the [`UserPermissionSource`] implementation returns; it is
/// public so a caller (the Desk payload in `ferris-api`) can show the same
/// rules the engine is enforcing.
#[must_use]
pub fn rules_for(user: &str) -> UserPermissionRules {
    SNAPSHOT
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(user)
        .map_or_else(|| Arc::clone(&NO_RULES), Arc::clone)
}

/// The port implementation: a snapshot lookup.
#[derive(Debug)]
struct SnapshotSource;

impl UserPermissionSource for SnapshotSource {
    fn rules_for(&self, user: &str) -> UserPermissionRules {
        rules_for(user)
    }
}

/// Reload every rule into the snapshot.
///
/// One engine `list` for the rows plus one list per tree `DocType` that a
/// `hide_descendants` rule names — never one query per rule, per user or per
/// row. A rule whose `hide_descendants` cannot be honoured is an error, so the
/// failure is loud at the refresh instead of an isolation that silently does
/// not exist.
///
/// # Errors
///
/// Database errors, and a [`ValidationError::Invalid`] for a `hide_descendants`
/// rule whose `allow` is not a tree `DocType`.
pub async fn refresh(db: &Db) -> Result<()> {
    let store = UserPermissionStore::new(db.clone());
    store.init().await?;
    let mut rules = store.list_all().await?;
    expand_descendants(&store.engine, &mut rules).await?;
    publish(rules);
    Ok(())
}

/// Install the store as the engine's source and keep the snapshot fresh.
///
/// Called once at boot by `ferris-api::authz::install_policies_with_role_permissions`.
/// The order matters: a failed load aborts the install (and boot) instead of
/// installing a source that would answer "unrestricted" for everybody.
///
/// # Errors
///
/// Whatever [`refresh`] returns.
pub async fn install(db: &Db) -> Result<()> {
    refresh(db).await?;
    set_user_permission_source(Arc::new(SnapshotSource));
    register_hooks(db);
    Ok(())
}

/// Forget every loaded rule. Tests use it; a server keeps the snapshot.
pub fn clear() {
    SNAPSHOT
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    ferris_core::permissions::clear_user_permission_source();
}

/// Replace the snapshot with `rules`, grouped by user.
fn publish(rules: Vec<UserPermissionRule>) {
    let mut by_user: HashMap<String, Vec<UserPermissionRule>> = HashMap::new();
    for rule in rules {
        by_user.entry(rule.user.clone()).or_default().push(rule);
    }
    let by_user = by_user
        .into_iter()
        .map(|(user, rules)| (user, UserPermissionRules::from(rules)))
        .collect();
    *SNAPSHOT
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = by_user;
}

/// Fill in [`UserPermissionRule::descendants`] for the tree rules.
async fn expand_descendants(
    engine: &DocumentEngine,
    rules: &mut [UserPermissionRule],
) -> Result<()> {
    // One walk per allowed DocType, not per rule: several users may be limited
    // to different values of the same tree.
    let mut children: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
    for rule in rules.iter_mut().filter(|rule| rule.hide_descendants) {
        if !children.contains_key(&rule.allow) {
            let tree = load_tree(engine, &rule.allow).await?;
            children.insert(rule.allow.clone(), tree);
        }
        if let Some(tree) = children.get(&rule.allow) {
            rule.descendants = descendants_of(tree, &rule.for_value);
        }
    }
    Ok(())
}

/// Read one tree `DocType` as a `parent → children` map.
async fn load_tree(engine: &DocumentEngine, doctype: &str) -> Result<HashMap<String, Vec<String>>> {
    let meta = ferris_schema::get_doctype(doctype).ok_or_else(|| rejected(doctype, None))?;
    if !meta.is_tree {
        return Err(rejected(doctype, Some(meta.name.as_str())));
    }
    let parent_field = meta
        .tree_parent_field
        .clone()
        .unwrap_or_else(|| DEFAULT_TREE_PARENT.to_string());
    let ctx = system_context(&meta);
    let query = ListQuery {
        limit: Some(u32::MAX),
        ..ListQuery::default()
    };
    let (documents, _) = engine.list(&ctx, &query).await?;
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for document in documents {
        let name = document
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if name.is_empty() {
            continue;
        }
        let parent = document
            .get(&parent_field)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !parent.is_empty() {
            children.entry(parent).or_default().push(name);
        }
    }
    Ok(children)
}

/// Every descendant of `root`, depth-first, without revisiting a node.
///
/// A cycle in the tree (a row that is its own ancestor) is stopped by the
/// `seen` set instead of looping forever.
fn descendants_of(children: &HashMap<String, Vec<String>>, root: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_string()];
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    seen.insert(root.to_string());
    while let Some(current) = stack.pop() {
        for child in children.get(&current).into_iter().flatten() {
            if seen.insert(child.clone()) {
                out.push(child.clone());
                stack.push(child.clone());
            }
        }
    }
    out
}

/// The typed rejection for a `hide_descendants` rule that cannot be honoured.
fn rejected(doctype: &str, known: Option<&str>) -> Error {
    let message = match known {
        Some(_) => format!(
            "`hide_descendants` requires `{doctype}` to be a tree DocType; the rule was rejected \
             instead of being ignored"
        ),
        None => format!(
            "`hide_descendants` names `{doctype}`, which is not a registered DocType; the rule was \
             rejected instead of being ignored"
        ),
    };
    Error::Validation(ValidationError::Invalid(message))
}

/// Register the hooks that keep the snapshot current and validate new rows.
///
/// `OnUpdate` covers insert (the engine runs it after `AfterInsert`) and update;
/// `OnTrash` runs after the delete commits. Both refresh from the database, so
/// a failed write never changes the snapshot. A refresh failure is logged and
/// the previous snapshot is kept — stale rules keep restricting rather than
/// silently widening access.
fn register_hooks(db: &Db) {
    let db = Arc::new(db.clone());
    for point in [HookPoint::OnUpdate, HookPoint::OnTrash] {
        let db = Arc::clone(&db);
        HookRegistry::replace_global(
            HOOK_REFRESH,
            DOCTYPE.to_string(),
            point,
            Box::new(move |_ctx, _doc| {
                let db = Arc::clone(&db);
                Box::pin(async move {
                    if let Err(error) = refresh(&db).await {
                        tracing::warn!(
                            "user-permission: the rule cache could not be refreshed: {error}"
                        );
                    }
                    Ok(())
                })
            }),
            0,
            None,
        );
    }

    HookRegistry::replace_global(
        HOOK_VALIDATE,
        DOCTYPE.to_string(),
        HookPoint::BeforeValidate,
        Box::new(|_ctx, document| {
            // Copy the two values out before the future: a `&dyn DocumentHook`
            // borrow cannot outlive the call that hands it over.
            let hide = document
                .get_field("hide_descendants")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let allow = document
                .get_field("allow")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Box::pin(async move { validate_rule_values(hide, &allow) })
        }),
        0,
        None,
    );
}

/// Reject `hide_descendants` on a target that is not a tree `DocType`.
fn validate_rule_values(hide: bool, allow: &str) -> Result<()> {
    if !hide {
        return Ok(());
    }
    if allow.is_empty() {
        return Err(Error::Validation(ValidationError::Invalid(
            "`hide_descendants` needs `allow` to name a DocType".to_string(),
        )));
    }
    match ferris_schema::get_doctype(allow) {
        Some(meta) if meta.is_tree => Ok(()),
        other => Err(rejected(
            allow,
            other.as_ref().map(|meta| meta.name.as_str()),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferris_document::EngineContext;
    use ferris_schema::{DoctypeMeta, FieldMeta};

    /// The snapshot is process-wide; these tests serialise on it.
    static SERIALISE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct Guard {
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            clear();
        }
    }

    fn guard() -> Guard {
        let guard = SERIALISE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        clear();
        Guard { _guard: guard }
    }

    fn tree_meta() -> DoctypeMeta {
        DoctypeMeta {
            name: "D1Tree".to_string(),
            is_tree: true,
            tree_parent_field: Some("parent".to_string()),
            fields: vec![
                FieldMeta::link("parent", "D1Tree"),
                FieldMeta::data("title"),
            ],
            ..Default::default()
        }
    }

    fn plain_meta() -> DoctypeMeta {
        DoctypeMeta {
            name: "D1Plain".to_string(),
            fields: vec![FieldMeta::data("title")],
            ..Default::default()
        }
    }

    fn system() -> ferris_core::User {
        let mut user = ferris_core::User::system_user(
            "Administrator".to_string(),
            "Administrator".to_string(),
        );
        user.id = "Administrator".to_string();
        user.roles = vec![ferris_core::permissions::SYSTEM_MANAGER_ROLE.to_string()];
        user
    }

    #[tokio::test]
    async fn upsert_round_trips_and_refresh_publishes_by_user() {
        let _guard = guard();
        let db = ferris_db::memory_db().await.unwrap();
        let store = UserPermissionStore::new(db.clone());
        store.init().await.unwrap();

        let rule = UserPermissionRule::new("ada@local", "Customer", "CUST-A")
            .with_apply_to_all_doctypes(true);
        store.upsert(&rule).await.unwrap();
        // Same natural key: overwrite, not duplicate.
        store.upsert(&rule).await.unwrap();
        assert_eq!(store.list_all().await.unwrap().len(), 1);
        assert_eq!(store.list_for_user("ada@local").await.unwrap().len(), 1);
        assert!(store.list_for_user("bob@local").await.unwrap().is_empty());

        refresh(&db).await.unwrap();
        let installed = rules_for("ada@local");
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].for_value, "CUST-A");
        assert!(installed[0].apply_to_all_doctypes);
        assert!(rules_for("bob@local").is_empty());

        store
            .delete("ada@local", "Customer", "CUST-A")
            .await
            .unwrap();
        refresh(&db).await.unwrap();
        assert!(rules_for("ada@local").is_empty());
    }

    /// `hide_descendants` on a tree expands to its descendants, so the per-row
    /// check later sees a plain value list.
    #[tokio::test]
    async fn hide_descendants_expands_a_tree() {
        let _guard = guard();
        let meta = tree_meta();
        ferris_schema::register_doctype(meta.clone());
        let db = ferris_db::memory_db().await.unwrap();
        let engine = DocumentEngine::new(db.clone());
        engine.ensure_table("D1Tree").await.unwrap();
        let ctx = EngineContext::new("D1Tree", "")
            .with_meta(&meta)
            .with_user(&system())
            .with_explicit_name();
        engine
            .create(
                &ctx,
                ferris_core::value!({ "name": "ROOT", "title": "root" }),
            )
            .await
            .unwrap();
        engine
            .create(
                &ctx,
                ferris_core::value!({ "name": "CHILD", "title": "child", "parent": "ROOT" }),
            )
            .await
            .unwrap();
        engine
            .create(
                &ctx,
                ferris_core::value!({ "name": "GRANDCHILD", "title": "g", "parent": "CHILD" }),
            )
            .await
            .unwrap();

        let store = UserPermissionStore::new(db.clone());
        store.init().await.unwrap();
        store
            .upsert(
                &UserPermissionRule::new("ada@local", "D1Tree", "ROOT").with_hide_descendants(true),
            )
            .await
            .unwrap();

        refresh(&db).await.unwrap();
        let installed = rules_for("ada@local");
        assert_eq!(installed.len(), 1);
        assert!(installed[0].allows("CHILD"));
        assert!(installed[0].allows("GRANDCHILD"));
        assert!(!installed[0].allows("OTHER"));
    }

    /// A `hide_descendants` rule that cannot be honoured is rejected, not
    /// ignored: the load fails with a message naming the `DocType`.
    #[tokio::test]
    async fn hide_descendants_on_a_non_tree_is_rejected() {
        let _guard = guard();
        ferris_schema::register_doctype(plain_meta());
        let db = ferris_db::memory_db().await.unwrap();
        let store = UserPermissionStore::new(db.clone());
        store.init().await.unwrap();
        store
            .upsert(
                &UserPermissionRule::new("ada@local", "D1Plain", "X").with_hide_descendants(true),
            )
            .await
            .unwrap();

        let error = refresh(&db)
            .await
            .expect_err("a non-tree rule must be rejected");
        let message = error.to_string();
        assert!(message.contains("D1Plain"), "{message}");
        assert!(message.contains("hide_descendants"), "{message}");
    }

    /// The same rule is rejected at write time, so the operator gets a `400`
    /// instead of storing a rule the engine cannot honour.
    #[test]
    fn validation_rejects_hide_descendants_without_a_tree() {
        ferris_schema::register_doctype(plain_meta());
        let error =
            validate_rule_values(true, "D1Plain").expect_err("a non-tree target must be rejected");
        assert!(matches!(error, Error::Validation(_)), "{error:?}");

        // No flag, no complaint.
        validate_rule_values(false, "D1Plain").unwrap();
    }
}
