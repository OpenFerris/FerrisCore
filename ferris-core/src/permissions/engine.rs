use super::permission::{OWNER_CONDITION, Permission};
use super::policy::{Effect, Policy, PolicyStatement};
use super::principal::{Principal, PrincipalType};
use super::resource::Resource;
use crate::error::AuthzError;
use crate::{Result, Value};
use std::collections::HashMap;

/// Permission decision
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    NotApplicable,
}

/// The shape of a `Read` grant over a whole DocType.
///
/// [`Decision`] answers one resource; this answers "which rows of this DocType",
/// which is what a list query needs *before* it can page. See
/// [`PermissionEngine::read_scope`] for the analysis and for why the
/// row-by-row case is the fallback rather than the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadScope {
    /// Every row is readable: no `WHERE` term and no per-row check are needed.
    All,
    /// Only rows whose `owner` column is the caller: `owner = ?` in SQL.
    Owner,
    /// The grant is scoped to some rows only, so each row is evaluated.
    PerRow,
    /// No row is readable: the list is empty and no query needs to run.
    None,
}

/// How much of a DocType one statement reaches, once it already matches the
/// action, a principal and the DocType.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// Every row of the DocType.
    EveryRow,
    /// Only the rows whose `owner` column is the caller.
    OwnedByCaller,
    /// Some rows that the caller cannot name ahead of time.
    SomeRows,
}

/// Whether `pattern` can match a *document* of `doctype`.
///
/// This has to agree with [`PermissionEngine::resource_matches`] on which
/// resources reach a document at all, because a grant the checker never matches
/// on a document is not a read grant. A document resource is
/// `Resource::new(doctype, name)` — that is what `ferris-document`'s `can_read`
/// builds — so only two patterns reach one:
///
/// * `resource_type == doctype`, with a wildcard or empty `name`;
/// * `resource_type == "*"` with a wildcard or empty `name`, which is what
///   `apply_to_all_doctypes` emits.
///
/// `resource_type == "doctype"` does **not** qualify, and this is worth being
/// explicit about because [`doc_type_policy`] emits that resource alongside the
/// document one: it names the *DocType* as a thing to authorize, so it matches
/// `Resource::new("doctype", "ToDo")` and nothing else. Counting it would let a
/// DocType-level grant — for example a rights check over the schema — decide
/// which *documents* a caller may list.
fn reaches_doctype(pattern: &Resource, doctype: &str) -> bool {
    let ty = pattern.resource_type.as_str();
    ty == "*" || ty == doctype
}

/// Permission engine
#[derive(Debug, Clone)]
pub struct PermissionEngine {
    policies: Vec<Policy>,
    role_hierarchy: HashMap<String, Vec<String>>,
}

impl PermissionEngine {
    pub fn new() -> Self {
        Self {
            policies: Vec::new(),
            role_hierarchy: HashMap::new(),
        }
    }

    pub fn add_policy(&mut self, policy: Policy) {
        self.policies.push(policy);
    }

    pub fn policies(&self) -> &[Policy] {
        &self.policies
    }

    /// How much of `doctype` these principals can `Read`, as a *complete* answer.
    ///
    /// [`Self::check`] answers one resource at a time, so a list query can only
    /// discover that it may read a row by reading it. Ferris needs the answer
    /// before the first row, because `LIMIT`/`OFFSET` applied before the
    /// permission filter shortens the page *and* makes `total` a count of rows
    /// the caller cannot see (the bug pinned by
    /// `list_hides_documents_the_caller_cannot_read` in
    /// `ferris-document/src/engine.rs`). Frappe has the same need and answers it
    /// with `get_permission_query_conditions` — which returns nothing at all for
    /// a full grant, `owner = %(user)s` for `if_owner`, and otherwise a snippet
    /// of SQL.
    ///
    /// This returns which of those three shapes the installed policy reduces to.
    /// The analysis is over statements, not probes: a probe on one resource
    /// cannot see a rule scoped to a single `name`, and inferring "reads
    /// everything" from one allowed row would hand a caller a whole DocType the
    /// moment any one row answered yes. So every statement that could match a
    /// row of the DocType is inspected, and anything that is neither universal
    /// nor owner-scoped falls back to [`ReadScope::PerRow`] — the answer
    /// [`Self::check`] would give row by row, so nothing is over-granted.
    ///
    /// `owner` is the caller's own id, used to recognise the owner-scoped shape.
    /// It is passed explicitly because the owner shortcut in
    /// `ferris_auth::permissions::check_permission` compares against the
    /// *user*, not against a role principal.
    pub fn read_scope(&self, principals: &[Principal], doctype: &str, owner: &str) -> ReadScope {
        let mut allow_every = false;
        let mut allow_some = false;
        let mut deny_every = false;
        let mut deny_some = false;

        for policy in &self.policies {
            for statement in &policy.statements {
                if !statement
                    .actions
                    .iter()
                    .any(|granted| granted.grants(Permission::Read))
                {
                    continue;
                }
                // Same principal matching as `check`, so the role hierarchy
                // applies here exactly as it does there.
                let principal_match = statement.principals.iter().any(|pattern| {
                    principals
                        .iter()
                        .any(|p| self.principal_matches(pattern, p))
                });
                if !principal_match {
                    continue;
                }
                // A statement about a different DocType says nothing about this
                // one.
                if !statement
                    .resources
                    .iter()
                    .any(|r| reaches_doctype(r, doctype))
                {
                    continue;
                }

                let shape = Self::reach_shape(statement, doctype, owner);
                match (statement.effect, shape) {
                    (Effect::Allow, Reach::EveryRow) => allow_every = true,
                    // An owner-scoped allow needs no separate flag: with nothing
                    // universal in play it lands on `Owner` below either way,
                    // because the owner shortcut grants the same rows.
                    (Effect::Allow, Reach::OwnedByCaller) => {}
                    (Effect::Allow, Reach::SomeRows) => allow_some = true,
                    (Effect::Deny, Reach::EveryRow) => deny_every = true,
                    (Effect::Deny, Reach::OwnedByCaller | Reach::SomeRows) => deny_some = true,
                }
            }
        }

        // A deny covering every row beats every allow: `check` returns `Deny` for
        // all of them, and the owner shortcut is never reached.
        if deny_every {
            return ReadScope::None;
        }
        // An allow that is not universal, or a deny that is not, makes the
        // decision a function of the row. The per-row path answers those exactly
        // because it is the same `check` call, one row at a time.
        if allow_some || deny_some {
            return ReadScope::PerRow;
        }
        if allow_every {
            return ReadScope::All;
        }
        // Nothing universal either way. The owner shortcut in
        // `ferris_auth::permissions::check_permission` grants `Read` on any row
        // whose `owner` column is the caller, so `owner = ?` is sufficient — and
        // with no other grant in play, exact.
        ReadScope::Owner
    }

    /// How much of the DocType one statement reaches, given it already matches
    /// the action, a principal and the DocType.
    fn reach_shape(statement: &PolicyStatement, doctype: &str, owner: &str) -> Reach {
        // A statement that names one concrete document is row-dependent whatever
        // else it carries, so this is checked first: an `if_owner` grant on a
        // single `name` still only reaches that row, and collapsing it to
        // `OwnedByCaller` would hand the caller every document they own.
        //
        // The `name` slot also means different things depending on the
        // `resource_type`. On a document resource (`resource_type == doctype`) a
        // concrete name identifies one row. On the `doctype` resource — the shape
        // `ferris-api`'s installer emits — the slot holds the *DocType*, which
        // `reaches_doctype` has already matched, so it names no row at all.
        if statement
            .resources
            .iter()
            .any(|r| r.resource_type == doctype && r.name != "*" && !r.name.is_empty())
        {
            return Reach::SomeRows;
        }

        // `is_owner = true` is `owner == caller`, which is exactly
        // `WHERE owner = ?`. Only the `true` form is indexable: `is_owner = false`
        // grants the rows the caller does *not* own, which is the complement of
        // `Owner` and has no `WHERE` term of this shape.
        match statement.conditions.get(OWNER_CONDITION) {
            Some(Value::Bool(true)) => return Reach::OwnedByCaller,
            Some(_) => return Reach::SomeRows,
            None => {}
        }
        // Any other condition makes the match a function of something the caller
        // cannot be indexed by.
        if !statement.conditions.is_empty() {
            return Reach::SomeRows;
        }

        let mut every_row = false;
        let mut owned_by_caller = false;
        for resource in &statement.resources {
            match &resource.owner {
                None => every_row = true,
                Some(scoped) if scoped == owner => owned_by_caller = true,
                // A resource pinned to some other owner names the rows this
                // caller does *not* own, which is row-dependent by definition.
                Some(_) => {}
            }
        }
        if every_row {
            Reach::EveryRow
        } else if owned_by_caller {
            Reach::OwnedByCaller
        } else {
            Reach::SomeRows
        }
    }

    /// Load policies from JSON: either a single policy object or an array of them.
    pub fn load_policy_json(&mut self, json: &str) -> Result<()> {
        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|e| AuthzError::InvalidPolicy(format!("JSON: {e}")))?;
        if value.is_array() {
            let policies: Vec<Policy> = serde_json::from_value(value)
                .map_err(|e| AuthzError::InvalidPolicy(format!("document: {e}")))?;
            self.policies.extend(policies);
        } else {
            let policy: Policy = serde_json::from_value(value)
                .map_err(|e| AuthzError::InvalidPolicy(format!("document: {e}")))?;
            self.policies.push(policy);
        }
        Ok(())
    }

    /// Evaluate a request against every principal the caller holds.
    ///
    /// Deny always wins: if any principal is explicitly denied, the decision is
    /// `Deny` even when another allows.
    ///
    /// [`Principal::everyone`] is appended unconditionally. `principal_matches`
    /// already accepts an `Everyone` pattern against *any* principal
    /// (`permissions.rs:437-440`), so on a path that supplies at least one
    /// principal an `All`-scoped grant (Frappe's `ALL_USER_ROLE`,
    /// `frappe/permissions.py:568`) needs nothing extra. The append is what
    /// makes it reachable on the paths that supply principals derived from roles
    /// alone, where a caller holding **no** role record produced an empty list and
    /// matched nothing at all. The one remaining
    /// `PermissionEngine::check_user` caller like that is
    /// `ferris-document`'s `check_permission`
    /// (`ferris-document/src/engine/support/perm.rs`), which passes
    /// `&user.roles` and no user principal. (The former `document_can` wrapper
    /// was deleted in DOC2, so authorization has a single implementation.)
    ///
    /// `RealtimeHandler::can_read_doctype` was the second such path until it
    /// gained the caller's user id and switched to asking the engine for a
    /// *scope* (`ferris-realtime/src/handler.rs:191`). `read_scope` matches an
    /// `Everyone` pattern against any principal it is given
    /// (`permissions.rs:437-440`), so a caller whose list holds only the pushed
    /// user id still reaches an `All` grant there without the append.
    ///
    /// Frappe expresses the same thing by handing `ALL_USER_ROLE` to everybody in
    /// `get_roles` (`frappe/permissions.py:568`), which Ferris has no equivalent
    /// hook for — so the principal is appended here instead.
    ///
    /// Anonymous callers never reach this. Every production entry point
    /// (`ferris_auth::permissions::check_permission`, and the engine's own
    /// anonymous branch at `ferris-document/src/engine.rs:869-879`) refuses
    /// before consulting the engine unless the context opted in, so
    /// `Everyone` means "every *authenticated* caller", as it does in Frappe.
    pub fn check_principals(
        &self,
        principals: &[Principal],
        action: Permission,
        resource: &Resource,
        context: &HashMap<String, Value>,
    ) -> Decision {
        let everyone = Principal::everyone();
        let mut allowed = false;
        let mut denied = false;
        for principal in principals.iter().chain(std::iter::once(&everyone)) {
            match self.check(principal, action, resource, context) {
                Decision::Allow => allowed = true,
                Decision::Deny => denied = true,
                Decision::NotApplicable => {}
            }
        }
        if denied {
            Decision::Deny
        } else if allowed {
            Decision::Allow
        } else {
            Decision::NotApplicable
        }
    }

    /// Evaluate a request against all of the principal's roles.
    ///
    /// Deny always wins: if any role is explicitly denied, the decision is
    /// `Deny` even when another role allows.
    pub fn check_user(
        &self,
        roles: &[String],
        action: Permission,
        resource: &Resource,
        context: &HashMap<String, Value>,
    ) -> Decision {
        let principals: Vec<Principal> = roles.iter().map(|role| Principal::role(role)).collect();
        self.check_principals(&principals, action, resource, context)
    }

    pub fn set_role_hierarchy(&mut self, hierarchy: HashMap<String, Vec<String>>) {
        self.role_hierarchy = hierarchy;
    }

    pub fn check(
        &self,
        principal: &Principal,
        action: Permission,
        resource: &Resource,
        context: &HashMap<String, Value>,
    ) -> Decision {
        let mut explicit_allow = false;
        let mut explicit_deny = false;

        for policy in &self.policies {
            for statement in &policy.statements {
                // The `Manage` shorthand is expanded here, at decision time,
                // not as a hidden wildcard inside `matches`.
                let action_applies = statement
                    .actions
                    .iter()
                    .any(|granted| granted.grants(action));
                if action_applies && self.matches_context(statement, principal, resource, context) {
                    match statement.effect {
                        Effect::Allow => explicit_allow = true,
                        Effect::Deny => explicit_deny = true,
                    }
                }
            }
        }

        if explicit_deny {
            Decision::Deny
        } else if explicit_allow {
            Decision::Allow
        } else {
            Decision::NotApplicable
        }
    }

    /// Matches everything about a statement except the action list: principal,
    /// resource and conditions.
    fn matches_context(
        &self,
        statement: &PolicyStatement,
        principal: &Principal,
        resource: &Resource,
        context: &HashMap<String, Value>,
    ) -> bool {
        let principal_match = statement
            .principals
            .iter()
            .any(|p| self.principal_matches(p, principal));
        if !principal_match {
            return false;
        }

        if !statement
            .resources
            .iter()
            .any(|r| self.resource_matches(r, resource))
        {
            return false;
        }

        for (key, expected) in &statement.conditions {
            if let Some(actual) = context.get(key) {
                if !self.value_matches(actual, expected) {
                    return false;
                }
            } else {
                return false;
            }
        }

        true
    }

    fn principal_matches(&self, pattern: &Principal, principal: &Principal) -> bool {
        if pattern.principal_type == PrincipalType::Everyone {
            return true;
        }

        if pattern.principal_type != principal.principal_type {
            return false;
        }

        if pattern.identifier == "*" {
            return true;
        }

        if pattern.identifier == principal.identifier {
            return true;
        }

        if pattern.principal_type == PrincipalType::Role {
            if let Some(parents) = self.role_hierarchy.get(&principal.identifier) {
                return parents.contains(&pattern.identifier);
            }
        }

        false
    }

    pub(super) fn resource_matches(&self, pattern: &Resource, resource: &Resource) -> bool {
        if pattern.resource_type != "*" && pattern.resource_type != resource.resource_type {
            return false;
        }

        if pattern.name != "*" && pattern.name != resource.name {
            return false;
        }

        if let Some(owner) = &pattern.owner {
            if resource.owner.as_ref() != Some(owner) {
                return false;
            }
        }

        true
    }

    fn value_matches(&self, actual: &Value, expected: &Value) -> bool {
        actual == expected
    }
}

impl Default for PermissionEngine {
    fn default() -> Self {
        let mut engine = Self::new();
        engine.add_policy(Policy::default_document_permissions());
        engine
    }
}
