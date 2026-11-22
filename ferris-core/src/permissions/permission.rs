use super::engine::{Decision, PermissionEngine, ReadScope};
use super::principal::Principal;
use super::resource::Resource;
use super::user_permission::{has_user_permissions_for, value_denied_by_user_permissions};
use crate::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// Permission actions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Read,
    Write,
    Create,
    Delete,
    Submit,
    Cancel,
    Amend,
    Print,
    Email,
    Share,
    Report,
    Import,
    Export,
    Manage,
}

impl Permission {
    /// Does granting `self` also cover the requested `action`?
    ///
    /// `Permission::Manage` is an **explicit shorthand** for "every action".
    /// It lives here — the single named place — so the wildcard is a
    /// deliberate, documented behavior instead of a hidden equality quirk in
    /// the statement matcher. A policy action list must match the
    /// requested action exactly *or* contain `Manage`.
    pub fn grants(&self, action: Permission) -> bool {
        *self == action || *self == Permission::Manage
    }

    pub fn all() -> Vec<Permission> {
        vec![
            Permission::Read,
            Permission::Write,
            Permission::Create,
            Permission::Delete,
            Permission::Submit,
            Permission::Cancel,
            Permission::Amend,
            Permission::Print,
            Permission::Email,
            Permission::Share,
            Permission::Report,
            Permission::Import,
            Permission::Export,
            Permission::Manage,
        ]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Permission::Read => "read",
            Permission::Write => "write",
            Permission::Create => "create",
            Permission::Delete => "delete",
            Permission::Submit => "submit",
            Permission::Cancel => "cancel",
            Permission::Amend => "amend",
            Permission::Print => "print",
            Permission::Email => "email",
            Permission::Share => "share",
            Permission::Report => "report",
            Permission::Import => "import",
            Permission::Export => "export",
            Permission::Manage => "manage",
        }
    }

    /// Parse the wire spelling of an action (lowercase, surrounding space
    /// ignored). Private so the table has exactly one home without an inherent
    /// `from_str` shadowing `std::str::FromStr` — call `"…".parse::<Permission>()`.
    fn parse(s: &str) -> Option<Self> {
        match s {
            "read" => Some(Permission::Read),
            "write" => Some(Permission::Write),
            "create" => Some(Permission::Create),
            "delete" => Some(Permission::Delete),
            "submit" => Some(Permission::Submit),
            "cancel" => Some(Permission::Cancel),
            "amend" => Some(Permission::Amend),
            "print" => Some(Permission::Print),
            "email" => Some(Permission::Email),
            "share" => Some(Permission::Share),
            "report" => Some(Permission::Report),
            "import" => Some(Permission::Import),
            "export" => Some(Permission::Export),
            "manage" => Some(Permission::Manage),
            _ => None,
        }
    }
}

/// A string that names no [`Permission`], as reported by [`std::str::FromStr`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown permission: {0}")]
pub struct ParsePermissionError(pub String);

impl std::str::FromStr for Permission {
    type Err = ParsePermissionError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let wanted = s.trim().to_lowercase();
        Self::parse(&wanted).ok_or_else(|| ParsePermissionError(s.to_string()))
    }
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Permission rule: one `DocPerm` row.
///
/// `Default` is all-false, which is the right zero: a rule that names no right
/// grants nothing, so "start from nothing and turn on what you need" cannot
/// over-grant. Frappe spells the same thing with absent checkboxes in the
/// `DocPerm` child table (`frappe/core/doctype/docperm/docperm.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionRule {
    pub role: String,
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub delete: bool,
    pub submit: bool,
    pub cancel: bool,
    pub amend: bool,
    pub print: bool,
    pub email: bool,
    pub share: bool,
    pub report: bool,
    pub import: bool,
    pub export: bool,
    pub permlevel: u8,
    pub if_owner: bool,
    pub apply_to_all_doctypes: bool,
    /// App-defined permission types this row grants, by name
    /// (<https://docs.frappe.io/framework/permission-types>). Frappe's
    /// `Permission Type` rows add rights beyond the fixed read/write/… flags
    /// (`approve`, `verify`, …); a row with none grants no custom right.
    #[serde(default)]
    pub custom_permissions: Vec<String>,
}

impl PermissionRule {
    /// Does this `DocPerm` row carry `perm`?
    ///
    /// [`Permission::Manage`] deserves its own sentence, because the word means
    /// two *deliberately different* things in this module and the difference is
    /// a trap otherwise:
    ///
    /// * here it answers "does this row grant the `manage` shorthand", which
    ///   Frappe defines as read+write+delete;
    /// * on the policy side [`Permission::grants`] expands a granted `Manage`
    ///   into *every* action, so a policy listing `manage` covers email, import,
    ///   … as well.
    ///
    /// [`doc_type_policy`] keeps the two apart by iterating [`DOC_RIGHTS`],
    /// which omits `Manage`, so a three-right row never becomes a thirteen-right
    /// policy statement. The test `manage_is_rwdelete_on_a_row_and_everything_in_a_policy`
    /// pins both directions.
    pub fn check(&self, perm: Permission) -> bool {
        match perm {
            Permission::Read => self.read,
            Permission::Write => self.write,
            Permission::Create => self.create,
            Permission::Delete => self.delete,
            Permission::Submit => self.submit,
            Permission::Cancel => self.cancel,
            Permission::Amend => self.amend,
            Permission::Print => self.print,
            Permission::Email => self.email,
            Permission::Share => self.share,
            Permission::Report => self.report,
            Permission::Import => self.import,
            Permission::Export => self.export,
            Permission::Manage => self.read && self.write && self.delete,
        }
    }

    /// Does this row grant the custom permission type `perm_type`?
    ///
    /// Custom types are app-defined strings, so the comparison is by trimmed
    /// name (the same tolerance a stored `Permission Type` row deserves) rather
    /// than through [`Permission`], whose variants are fixed. A rule with an
    /// empty list grants none.
    #[must_use]
    pub fn grants_custom(&self, perm_type: &str) -> bool {
        let wanted = perm_type.trim();
        !wanted.is_empty()
            && self
                .custom_permissions
                .iter()
                .any(|granted| granted.trim() == wanted)
    }
}

/// Frappe's `ALL_USER_ROLE`. A `DocPerm` row naming it means "every signed-in
/// user", and Frappe does not make that happen by storing it on each user:
/// `get_roles` appends it to every resolved role list
/// (`frappe/permissions.py:568`). Ferris has no role-resolution hook to append
/// to, so it maps onto [`Principal::everyone`] instead — see
/// [`PermissionEngine::check_principals`], which is what makes that reachable.
pub const ALL_USER_ROLE: &str = "All";

/// The framework's superuser role. It bypasses every engine decision in
/// [`PermissionEngine::default`], `ferris_auth::permissions::check_permission`
/// and `ferris-realtime`'s room gate, and — with the `"*", "*"` grant in
/// `default_document_permissions` — it would read everything even without the
/// shortcuts (`ferris-realtime/src/handler.rs:182-190`).
pub const SYSTEM_MANAGER_ROLE: &str = "System Manager";

/// The roles Frappe ships with, in the same hierarchy order.
///
/// Often called the *seeded* app roles: Frappe installs them as `Role`
/// documents (`frappe/core/doctype/role/roles.txt`), so a `DocPerm` row may
/// name them and the desk can render them before any app defines anything.
/// Ferris has no `Role` doctype — roles are free-form strings carried on a
/// user and matched by name — so the catalogue is deliberately *not* enforced:
/// an app may define and use its own roles, as in Frappe. What the list gives
/// Ferris is the same scaffold: a stable name for each framework role,
/// something for the Role Permissions Builder surface to offer as starting
/// choices, and a definition of [`is_standard_role`] to recognise the
/// framework's own names (e.g. to refuse assigning "All" to a user, which
/// Frappe forbids because it is woven into every role list).
pub const STANDARD_ROLES: &[&str] = &[
    "Administrator",
    SYSTEM_MANAGER_ROLE,
    ALL_USER_ROLE,
    "Guest",
    "Desk User",
    "Website User",
];

/// Whether `role` is one of the framework's seeded catalogue names.
pub fn is_standard_role(role: &str) -> bool {
    STANDARD_ROLES.contains(&role)
}

/// Context key carrying "the caller owns this document".
///
/// This is the only way a policy can express Frappe's `DocPerm.if_owner`
/// (`frappe/permissions.py:319-341`): the rule grants a right *to the owner and
/// nobody else*, and a statement cannot say that with a role principal and a
/// resource alone, because [`Resource::owner`] identifies the document's owner,
/// not the requester's.
///
/// Nothing populated it before this key existed. [`PermissionEngine::check`]
/// reads its `conditions` map against the caller's context
/// (`permissions.rs:391-399`), and every production call site passed an empty
/// map — so a conditioned statement matched *nothing at all*, silently. A
/// condition was therefore a way to write a rule that could never fire.
pub const OWNER_CONDITION: &str = "is_owner";

/// Whether `user` may access fields declared at `permlevel` for `action`.
///
/// Frappe's `get_permlevel_access` (`frappe/permissions.py`) is the reference:
/// the field levels a caller may touch are exactly those named by a `DocPerm`
/// row for one of the caller's roles that carries the right. Two things follow
/// from how the document engine authorizes a whole document *before* it looks at
/// fields:
///
/// * **level 0 is the document itself.** `check_permission` already admitted the
///   caller (role rule, owner shortcut, or System Manager), so a level-0 field
///   is theirs to read/write. Only a level *above* 0 needs a matching field
///   rule — which is why this returns `true` for 0 rather than consulting rules.
/// * **System Manager sees every level**, the same shortcut every other engine
///   decision takes.
///
/// [`ALL_USER_ROLE`] is honoured as "every signed-in user", matching how
/// `doc_type_policy` maps it onto [`Principal::everyone`].
#[must_use]
pub fn has_permlevel_access(
    rules: &[PermissionRule],
    user: &crate::user::User,
    action: Permission,
    permlevel: u8,
) -> bool {
    if permlevel == 0 || user.has_role(SYSTEM_MANAGER_ROLE) {
        return true;
    }
    rules.iter().any(|rule| {
        rule.permlevel == permlevel
            && rule.check(action)
            && (rule.role == ALL_USER_ROLE || user.has_role(&rule.role))
    })
}

/// Process-wide permission engine used by [`check_permission`].
///
/// Having a single shared engine means policies registered at startup are
/// actually enforced; previously every call built a fresh default engine and
/// silently ignored configuration.
///
/// # Swap semantics
///
/// [`set_global_engine`] replaces the whole engine under one write lock, and
/// every reader takes a read lock, so a check observes either the old engine or
/// the new one in full — never a half-swapped policy set. There is no
/// generation counter: a check that started before the swap and reads again
/// after it is simply re-evaluated against the new engine, which is the
/// behaviour a hot reload wants.
///
/// Two consequences callers must respect:
///
/// * install the engine **at boot, before requests are served** — a check that
///   runs before installation sees [`PermissionEngine::default`], not your
///   policy;
/// * the state is process-wide, so tests that install a custom engine must
///   serialise against each other (a plain `static Mutex` in the test module is
///   enough; `ferris-document` does this) or the suite will read another
///   test's engine.
static GLOBAL_ENGINE: OnceLock<RwLock<PermissionEngine>> = OnceLock::new();

fn global_engine() -> &'static RwLock<PermissionEngine> {
    GLOBAL_ENGINE.get_or_init(|| RwLock::new(PermissionEngine::default()))
}

/// Replace the process-wide permission engine (typically called once at boot).
///
/// The swap happens under one write lock; see [`GLOBAL_ENGINE`] for what a
/// concurrent check may observe and for the test-isolation rule.
pub fn set_global_engine(engine: PermissionEngine) {
    *global_engine().write().unwrap_or_else(|e| e.into_inner()) = engine;
}

/// Get the global permission engine for direct policy evaluation.
pub fn global_engine_ref() -> &'static RwLock<PermissionEngine> {
    global_engine()
}

/// The principals every engine entry point must ask about `user`: one per role,
/// plus the user itself.
///
/// `check_permission` and `read_scope` have to answer about *exactly* the same
/// principal list, or `read_scope` could promise a `WHERE`-able shape for a
/// caller `check_permission` would never admit. Building the list here makes
/// that invariant structural instead of a comment, and gives the one other
/// place that assembles it by hand (`ferris-realtime`'s room gate) a single
/// function to call.
#[must_use]
pub fn caller_principals(user: &crate::user::User) -> Vec<Principal> {
    principals_for(&user.id, &user.roles)
}

/// The roles-plus-user list, from the raw pair.
///
/// This is the single construction point; [`caller_principals`] is the
/// `&User` convenience over it, so `ferris-realtime` (which has an id and a role
/// list, not a `User`) does not need a second copy of the rule.
#[must_use]
pub fn principals_for(user_id: &str, roles: &[String]) -> Vec<Principal> {
    // Roles *and* the user itself: a per-user statement used to be as
    // unreachable as an `Everyone` one, because only `Principal::role(..)` was
    // ever constructed and `Principal::user(..)` matched nothing.
    let mut principals: Vec<Principal> = roles.iter().map(|r| Principal::role(r)).collect();
    principals.push(Principal::user(user_id));
    principals
}

/// Build the context a permission check is evaluated against.
///
/// The one key that exists is [`OWNER_CONDITION`], and it is what makes
/// Frappe's `DocPerm.if_owner` expressible: a statement conditioned on
/// `is_owner` grants its right to the document's owner and to nobody else.
/// [`PermissionEngine::check`] treats a missing condition key as *no match*, so
/// a conditioned statement is unreachable unless the key is populated here.
fn check_context(user: &crate::user::User, resource: &Resource) -> HashMap<String, Value> {
    // A blank owner is how the document engine spells "there is no stored
    // owner" (`trusted_owner.unwrap_or("")`), so it must not compare equal to a
    // user whose id happens to be blank. Frappe has the same edge and handles it
    // by lowercasing both sides and letting an empty owner match only an empty
    // user, which cannot happen (`frappe/permissions.py:241`).
    let is_owner = matches!(&resource.owner, Some(owner) if !owner.is_empty() && *owner == user.id);
    HashMap::from([(OWNER_CONDITION.to_string(), Value::Bool(is_owner))])
}

/// Check permission for a user using the global Cedar policy engine.
///
/// Deny always wins over allow; absence of a matching statement means no access
/// (default-deny). Owners may read/write/amend their own documents.
///
/// This is the single authorisation entry point the document engine calls for
/// every read and write, which is why it lives in `ferris-core` next to the
/// engine it drives: `ferris-document` cannot depend on `ferris-auth`.
pub fn check_permission(user: &crate::user::User, resource: &Resource, action: Permission) -> bool {
    if user.has_role(SYSTEM_MANAGER_ROLE) {
        return true;
    }

    // `User Permission` narrows *which values* a user reaches; the role rules
    // below grant the action on them. This is the one case core can answer
    // without metadata: the document **is** the allowed `DocType`, so its name
    // is the value. Link fields elsewhere are enforced by the document engine
    // (`ferris_document`), which can see the field list.
    if value_denied_by_user_permissions(&user.id, &resource.resource_type, &resource.name) {
        return false;
    }

    // Explicit policy evaluation. Deny always wins over allow; absence of a
    // matching statement means no access (default-deny).
    let mut allowed = false;
    let mut denied = false;
    {
        let engine = global_engine_ref()
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let principals = caller_principals(user);
        let context = check_context(user, resource);
        match engine.check_principals(&principals, action, resource, &context) {
            Decision::Allow => allowed = true,
            Decision::Deny => denied = true,
            Decision::NotApplicable => {}
        }
    }

    if denied {
        return false;
    }
    if allowed {
        return true;
    }

    // Owners may read and write their own document, but never gain elevated
    // actions (delete/submit/cancel/import/export/…).
    if matches!(
        action,
        Permission::Read | Permission::Write | Permission::Amend
    ) {
        if let Some(owner) = &resource.owner {
            if user.id == *owner {
                return true;
            }
        }
    }

    false
}

/// Which rows of `doctype` this user can `Read`, as a `WHERE`-able shape.
///
/// The list counterpart to [`check_permission`], and it mirrors that function's
/// two short-circuits rather than re-deriving them: `System Manager` reads
/// everything, and the owner shortcut decides the remaining shape. Anything the
/// analysis cannot settle comes back [`ReadScope::PerRow`], which makes the
/// caller fall back to calling [`check_permission`] once per row — i.e. to
/// today's behaviour.
///
/// The principal list comes from [`caller_principals`], the same helper
/// [`check_permission`] uses, so the two cannot drift.
pub fn read_scope(user: &crate::user::User, doctype: &str) -> ReadScope {
    if user.has_role(SYSTEM_MANAGER_ROLE) {
        return ReadScope::All;
    }
    // A `User Permission` turns the read decision into a function of the row's
    // own values, which this function cannot see. `All` would let a list query
    // page over rows the per-row check refuses (`LIMIT` applied before the
    // filter), so the answer is `PerRow`; the engine then evaluates every row,
    // including the `Link` fields the rule names. With no rule installed the
    // classification is exactly the previous one.
    if has_user_permissions_for(&user.id, doctype) {
        return ReadScope::PerRow;
    }
    let principals = caller_principals(user);
    let engine = global_engine_ref()
        .read()
        .unwrap_or_else(|e| e.into_inner());
    engine.read_scope(&principals, doctype, &user.id)
}
