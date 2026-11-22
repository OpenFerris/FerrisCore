//! Per-user value restrictions — the `User Permission` capability (FRA-03).
//!
//! Frappe lets an operator narrow a user to one *value* of a linked `DocType`
//! (`User Permission {user, allow, for_value}`): "this clerk sees only
//! `Customer A`". The `DocType` and the Desk have always existed here, but nothing
//! read them, so an operator could configure an isolation that did not exist.
//! This module is the model and the port; `ferris-auth` owns the storage and the
//! refresh, `ferris-document` enforces it on reads and writes.
//!
//! # Why the rules live in `ferris-core`
//!
//! `ferris-document` authorises every read and write through
//! [`ferris_core::permissions::check_permission`] and
//! [`ferris_core::permissions::read_scope`], and it must not depend on
//! `ferris-auth` (that would invert the dependency and drag the session/OAuth
//! stack into the engine). So the *shape* of a rule and the process-wide port
//! live here, and the database-backed source is installed by the crate that
//! owns the `User Permission` table.
//!
//! # Why the port is synchronous
//!
//! The document engine's per-row decision is a synchronous predicate
//! ([`check_permission`], [`read_scope`]), called once per row in a list. A
//! database read there would be one query per row. The installed source
//! therefore serves an in-memory snapshot — one query per refresh, not per
//! request and certainly not per row — and the writer that changes a rule
//! refreshes it
//! (`ferris_auth::user_permission`).
//!
//! # Default is "no restriction"
//!
//! With no source installed (a test, a WASM guest, a CLI command that never
//! booted a server) every call returns an empty list and the previous behaviour
//! is kept exactly: role rules and ownership decide, and nothing is narrowed.
//! `ferris-auth` installs the real source at boot; the server is the component
//! that knows the database.
//!
//! [`check_permission`]: super::check_permission
//! [`read_scope`]: super::read_scope

use serde::{Deserialize, Serialize};
use std::sync::{Arc, LazyLock, RwLock};

/// One `User Permission` row, as the permission engine needs it.
///
/// The field names mirror the `DocType` so an operator reading the Desk and a
/// developer reading this struct see the same vocabulary. `allow` is the
/// **`DocType`** whose values are allowed (Frappe's `allow`), `for_value` is the
/// permitted value of it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserPermissionRule {
    /// The user the rule narrows.
    pub user: String,
    /// The `DocType` whose values are allowed.
    pub allow: String,
    /// The one allowed value of [`Self::allow`].
    pub for_value: String,
    /// Restrict only this `DocType` (a `DocType` with a `Link` to
    /// [`Self::allow`]). Empty/absent means "only when
    /// [`Self::apply_to_all_doctypes`] is set".
    #[serde(default)]
    pub applicable_for: Option<String>,
    /// Apply the rule to every `DocType` that links to [`Self::allow`].
    #[serde(default)]
    pub apply_to_all_doctypes: bool,
    /// For a tree `DocType`, allow the descendants of [`Self::for_value`] too.
    #[serde(default)]
    pub hide_descendants: bool,
    /// Descendants of [`Self::for_value`], resolved once by the loader when
    /// [`Self::hide_descendants`] is set.
    ///
    /// This is data, not policy: the tree walk needs the document table, which
    /// this crate does not have. Keeping the resolved list on the rule means the
    /// per-row check is a string comparison, never a query, and the loader is
    /// the one place that has to know a `DocType` is a tree.
    #[serde(default)]
    pub descendants: Vec<String>,
}

impl UserPermissionRule {
    /// A rule allowing exactly `for_value` of `allow` to `user`.
    #[must_use]
    pub fn new(
        user: impl Into<String>,
        allow: impl Into<String>,
        for_value: impl Into<String>,
    ) -> Self {
        Self {
            user: user.into(),
            allow: allow.into(),
            for_value: for_value.into(),
            ..Self::default()
        }
    }

    /// Narrow the rule to one `DocType`.
    #[must_use]
    pub fn with_applicable_for(mut self, doctype: impl Into<String>) -> Self {
        self.applicable_for = Some(doctype.into());
        self
    }

    /// Apply the rule to every `DocType` that links to [`Self::allow`].
    #[must_use]
    pub const fn with_apply_to_all_doctypes(mut self, apply: bool) -> Self {
        self.apply_to_all_doctypes = apply;
        self
    }

    /// Include the descendants of a tree `for_value`.
    #[must_use]
    pub const fn with_hide_descendants(mut self, hide: bool) -> Self {
        self.hide_descendants = hide;
        self
    }

    /// Attach the resolved descendants (the loader's expansion).
    #[must_use]
    pub fn with_descendants(mut self, descendants: Vec<String>) -> Self {
        self.descendants = descendants;
        self
    }

    /// Does this rule say anything about `doctype`?
    ///
    /// The target `DocType` itself always qualifies — a rule on `Customer` is
    /// what makes `Customer` rows readable by name. Any other `DocType` needs
    /// either an explicit [`Self::applicable_for`] or
    /// [`Self::apply_to_all_doctypes`]; otherwise the rule is only about
    /// [`Self::allow`], which is Frappe's semantics (a `User Permission` with
    /// neither flag restricts the linked `DocType`, not the link's holders).
    ///
    /// This is the *coarse* question `read_scope` can answer without metadata.
    /// Whether a concrete row satisfies the rule is decided by the document
    /// engine, which can see the `Link` fields.
    #[must_use]
    pub fn applies_to(&self, doctype: &str) -> bool {
        if self.allow == doctype {
            return true;
        }
        match self.applicable_for.as_deref() {
            Some(scoped) if !scoped.is_empty() => scoped == doctype,
            _ => self.apply_to_all_doctypes,
        }
    }

    /// Whether `value` is the allowed value (or one of its descendants).
    #[must_use]
    pub fn allows(&self, value: &str) -> bool {
        value == self.for_value || self.descendants.iter().any(|child| child == value)
    }
}

/// The rules of one user, shared so a lookup does not copy them.
pub type UserPermissionRules = Arc<[UserPermissionRule]>;

/// The process-wide source of installed rules.
///
/// Implemented by `ferris-auth` over its in-memory snapshot; see the module
/// docs for why the call is synchronous.
pub trait UserPermissionSource: Send + Sync {
    /// Every rule installed for `user`. An empty result means unrestricted.
    fn rules_for(&self, user: &str) -> UserPermissionRules;
}

/// The empty set, returned when no source is installed.
static NO_RULES: LazyLock<UserPermissionRules> =
    LazyLock::new(|| UserPermissionRules::from(Vec::new()));

/// The installed source, if any.
static SOURCE: LazyLock<RwLock<Option<Arc<dyn UserPermissionSource>>>> =
    LazyLock::new(|| RwLock::new(None));

/// Install the source that serves the rules.
///
/// Called once at boot by `ferris-auth` (`install`). Idempotent: a second call
/// replaces the source, which is what a hot reload needs.
pub fn set_user_permission_source(source: Arc<dyn UserPermissionSource>) {
    *SOURCE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(source);
}

/// Remove the installed source, restoring "no restriction".
///
/// Tests use it to leave the process as they found it; a server never needs it.
pub fn clear_user_permission_source() {
    *SOURCE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// Every rule installed for `user`, or an empty list when nothing restricts
/// them.
#[must_use]
pub fn user_permissions_for(user: &str) -> UserPermissionRules {
    let source = SOURCE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    source.map_or_else(|| Arc::clone(&NO_RULES), |source| source.rules_for(user))
}

/// Whether `user` has any rule that says something about `doctype`.
///
/// `read_scope` uses this to refuse `ReadScope::All`: with a rule in play the
/// decision is a function of the row, so the list must fall back to the per-row
/// check. A false answer keeps the previous classifications untouched.
#[must_use]
pub fn has_user_permissions_for(user: &str, doctype: &str) -> bool {
    let rules = user_permissions_for(user);
    rules.iter().any(|rule| rule.applies_to(doctype))
}

/// Whether the rules forbid the document `name` of `doctype` outright.
///
/// Only the case the core can answer without metadata: the document **is** the
/// allowed value's `DocType` (a `Customer` row), so its own name is the value to
/// compare. A blank name (a `DocType`-level probe, a create before naming) is
/// not denied — the link fields are checked by the engine on write, where they
/// exist.
///
/// Rules on the same `DocType` are alternatives, like Frappe's values for one
/// field: `Customer A` *or* `Customer B` means either row is readable.
#[must_use]
pub fn value_denied_by_user_permissions(user: &str, doctype: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let rules = user_permissions_for(user);
    let mut targeted = false;
    for rule in rules.iter().filter(|rule| rule.allow == doctype) {
        targeted = true;
        if rule.allows(name) {
            return false;
        }
    }
    targeted
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Static(Vec<UserPermissionRule>);

    impl UserPermissionSource for Static {
        fn rules_for(&self, user: &str) -> UserPermissionRules {
            UserPermissionRules::from(
                self.0
                    .iter()
                    .filter(|rule| rule.user == user)
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        }
    }

    /// The port is process-wide; a test that installs a source must not race
    /// another one in the same binary.
    static SERIALISE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct Installed {
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for Installed {
        fn drop(&mut self) {
            clear_user_permission_source();
        }
    }

    fn install(rules: Vec<UserPermissionRule>) -> Installed {
        let guard = SERIALISE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        set_user_permission_source(Arc::new(Static(rules)));
        Installed { _guard: guard }
    }

    #[test]
    fn no_source_means_no_restriction() {
        let _installed = install(Vec::new());
        assert!(user_permissions_for("ada@local").is_empty());
        assert!(!has_user_permissions_for("ada@local", "Customer"));
        assert!(!value_denied_by_user_permissions(
            "ada@local",
            "Customer",
            "Customer B"
        ));
    }

    #[test]
    fn a_rule_restricts_its_users_own_allow_doctype() {
        let _installed = install(vec![UserPermissionRule::new(
            "ada@local",
            "Customer",
            "Customer A",
        )]);
        assert!(has_user_permissions_for("ada@local", "Customer"));
        assert!(!has_user_permissions_for("ada@local", "ToDo"));
        assert!(value_denied_by_user_permissions(
            "ada@local",
            "Customer",
            "Customer B"
        ));
        assert!(!value_denied_by_user_permissions(
            "ada@local",
            "Customer",
            "Customer A"
        ));
        // A blank name is a DocType-level probe, not a row.
        assert!(!value_denied_by_user_permissions(
            "ada@local",
            "Customer",
            ""
        ));
    }

    #[test]
    fn representative_rows_are_alternatives_not_conjunctions() {
        let _installed = install(vec![
            UserPermissionRule::new("ada@local", "Customer", "Customer A"),
            UserPermissionRule::new("ada@local", "Customer", "Customer B"),
        ]);
        assert!(!value_denied_by_user_permissions(
            "ada@local",
            "Customer",
            "Customer B"
        ));
    }

    #[test]
    fn apply_to_all_doctypes_reaches_every_linking_doctype_but_not_others() {
        let _installed = install(vec![
            UserPermissionRule::new("ada@local", "Customer", "Customer A")
                .with_apply_to_all_doctypes(true),
        ]);
        assert!(has_user_permissions_for("ada@local", "Sales Order"));
        assert!(
            UserPermissionRule::new("ada@local", "Customer", "Customer A")
                .with_apply_to_all_doctypes(true)
                .applies_to("Sales Order")
        );
        assert!(!UserPermissionRule::new("ada@local", "Customer", "Customer A").applies_to("ToDo"));
    }

    #[test]
    fn applicable_for_narrows_the_rule_to_one_doctype() {
        let rule = UserPermissionRule::new("ada@local", "Customer", "Customer A")
            .with_applicable_for("Sales Order")
            .with_apply_to_all_doctypes(true);
        assert!(rule.applies_to("Sales Order"));
        assert!(rule.applies_to("Customer"));
        assert!(!rule.applies_to("Quotation"));
    }

    #[test]
    fn descendants_are_part_of_the_allowed_value() {
        let rule = UserPermissionRule::new("ada@local", "Customer Group", "Retail")
            .with_hide_descendants(true)
            .with_descendants(vec!["Retail/Shops".to_string()]);
        assert!(rule.allows("Retail/Shops"));
        assert!(!rule.allows("Wholesale"));
    }
}
