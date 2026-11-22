use super::permission::{ALL_USER_ROLE, OWNER_CONDITION, Permission, PermissionRule};
use super::principal::Principal;
use super::resource::Resource;
use crate::error::{AuthzError, SerializationError};
use crate::{Result, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Policy statement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyStatement {
    pub effect: Effect,
    pub principals: Vec<Principal>,
    pub actions: Vec<Permission>,
    pub resources: Vec<Resource>,
    pub conditions: HashMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    Allow,
    Deny,
}

impl PolicyStatement {
    pub fn allow(
        principals: Vec<Principal>,
        actions: Vec<Permission>,
        resources: Vec<Resource>,
    ) -> Self {
        Self {
            effect: Effect::Allow,
            principals,
            actions,
            resources,
            conditions: HashMap::new(),
        }
    }

    pub fn deny(
        principals: Vec<Principal>,
        actions: Vec<Permission>,
        resources: Vec<Resource>,
    ) -> Self {
        Self {
            effect: Effect::Deny,
            principals,
            actions,
            resources,
            conditions: HashMap::new(),
        }
    }

    pub fn with_condition(mut self, key: impl Into<String>, value: Value) -> Self {
        self.conditions.insert(key.into(), value);
        self
    }
}

/// Policy document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub version: String,
    pub statements: Vec<PolicyStatement>,
}

impl Policy {
    pub fn new(statements: Vec<PolicyStatement>) -> Self {
        Self {
            version: "2024-01-01".to_string(),
            statements,
        }
    }

    /// Parse a policy document from JSON.
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json)
            .map_err(|e| AuthzError::InvalidPolicy(format!("JSON: {e}")).into())
    }

    /// Serialise the policy document to pretty JSON.
    pub fn to_json_pretty(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(SerializationError::from)
            .map_err(Into::into)
    }

    /// Conservative baseline policy: **default-deny**.
    ///
    /// Only `System Manager` is granted broad access. There is intentionally no
    /// `Everyone`-read rule: every other role must be granted explicitly, which
    /// matches Frappe's docperm model (no rule means no access).
    pub fn default_document_permissions() -> Self {
        let statements = vec![PolicyStatement::allow(
            vec![Principal::role("System Manager")],
            Permission::all(),
            vec![Resource::new("*", "*")],
        )];

        Self::new(statements)
    }
}

/// The rights Frappe evaluates on every DocType, in `std_rights` order
/// (`frappe/permissions.py:14-29`).
///
/// `select` has no counterpart here: it is Frappe's narrow "may see this in a
/// list view" right, and Ferris already gets that for free because `list()`
/// drops the rows a caller may not read instead of failing the page.
///
/// [`Permission::Manage`] is deliberately absent. It is an *explicit* shorthand
/// that expands to every action at decision time (`permissions.rs:31-35`), and
/// `PermissionRule::check` reports it as set for any rule with read+write+delete
/// (`permissions.rs:499`). Emitting it here would turn every three-right rule
/// into a grant of all thirteen — the exact over-grant this translation exists
/// to avoid.
pub(super) const DOC_RIGHTS: [Permission; 13] = [
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
];

/// The DocType-level facts Frappe consults while evaluating a `DocPerm`.
///
/// Frappe zeroes two rights at check time regardless of what the row says: a
/// non-submittable DocType cannot be submitted, and one with `allow_import` off
/// cannot be imported (`frappe/permissions.py:252-258`, and again on the
/// DocType-level branch at `:160-166`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DocTypeCaps {
    pub submittable: bool,
    pub allow_import: bool,
}

/// Why a `PermissionRule` produced no policy statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Untranslated {
    /// `permlevel != 0`.
    ///
    /// Frappe's role-permission system ignores these outright: `is_perm_applicable`
    /// requires `permlevel == 0` (`frappe/permissions.py:318`). They belong to
    /// field-level permissions, which Ferris does not implement — and it is the
    /// reason ignoring them is *right* rather than lazy. Granting them as if
    /// they were document rights would hand over every field's worth of access.
    Permlevel,
    /// Every right the rule set was suppressed, or it set none to begin with.
    NoRights,
    /// An `if_owner` rule whose only right was `create`.
    ///
    /// Frappe exempts `create` from the owner clamp outright
    /// (`frappe/permissions.py:339`): there is no document yet to own, and it
    /// force-sets `create = 0` on the owner path as well
    /// (`frappe/permissions.py:269`). Frappe's own `DocPerm` fixture agrees —
    /// `"permlevel": 1` rules in `test_custom_docperm.py` pair `if_owner` with
    /// `read`.
    OwnerOnlyCreate,
    /// A rule with a blank role grants nothing, in Frappe too: `is_perm_applicable`
    /// tests `perm.role in roles`, and `""` is in nobody's role list.
    NoRole,
}

impl Untranslated {
    pub fn as_str(self) -> &'static str {
        match self {
            Untranslated::Permlevel => "permlevel is not 0 (field-level permission)",
            Untranslated::NoRights => "no right survived the DocType capability filter",
            Untranslated::OwnerOnlyCreate => {
                "if_owner carries no right but create, which if_owner never grants"
            }
            Untranslated::NoRole => "rule has no role",
        }
    }
}

/// A `PermissionRule` that could not be represented, kept so the caller can log
/// it rather than drop it silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedRule {
    pub role: String,
    pub rights: Vec<Permission>,
    pub reason: Untranslated,
}

/// The result of translating one DocType's `permissions` array.
#[derive(Debug, Clone, Default)]
pub struct DocTypePolicy {
    pub statements: Vec<PolicyStatement>,
    pub skipped: Vec<SkippedRule>,
}

/// Translate a DocType's declared `DocPerm` rows into policy statements.
///
/// Each rule becomes one statement, because a rule is already the unit Frappe
/// evaluates: *role* × *rights* × *doctype*, with `if_owner` as a restriction
/// on top. Splitting the rights into separate statements would not change any
/// decision — statements are OR'd for `allow` and AND'd for `deny`
/// (`permissions.rs:340-366`) — and merging different roles into one statement
/// would, so the split is by role.
///
/// # The resource shape
///
/// A statement carries **two** resources for the same DocType, and this is
/// deliberate rather than redundant. Ferris identifies a document under two
/// different shapes:
///
/// - `(docname, doctype)` — the REST path, `Resource::new(&ctx.doctype, name)`
///   in `ferris-document/src/engine.rs`, which is the only shape §C2's fix
///   passes a real document name to.
/// - `("doctype", doctype)` — the two paths that ask "may this role touch this
///   DocType at all", the typed `Document` bridge in
///   `ferris-document/src/document.rs:283` and the realtime subscribe check in
///   `ferris-realtime/src/handler.rs:149`.
///
/// `Resource` matching takes `resource_type` and `name` separately
/// (`permissions.rs:430-446`), so a statement naming only one of the two shapes
/// reaches only half the framework. Both go in the same statement because
/// `matches_context` accepts a resource list where **any** match suffices
/// (`permissions.rs:387`).
pub fn doc_type_policy(
    doctype: &str,
    rules: &[PermissionRule],
    caps: DocTypeCaps,
) -> DocTypePolicy {
    let mut out = DocTypePolicy::default();

    for rule in rules {
        if rule.permlevel != 0 {
            out.skipped.push(SkippedRule {
                role: rule.role.clone(),
                rights: Vec::new(),
                reason: Untranslated::Permlevel,
            });
            continue;
        }

        if rule.role.trim().is_empty() {
            out.skipped.push(SkippedRule {
                role: rule.role.clone(),
                rights: Vec::new(),
                reason: Untranslated::NoRole,
            });
            continue;
        }

        let mut rights: Vec<Permission> = DOC_RIGHTS
            .iter()
            .copied()
            .filter(|right| rule.check(*right))
            .collect();

        // Frappe's per-DocType suppression, applied as "the right is not
        // granted" rather than as a second deny statement: deny is a *global*
        // veto in this engine (`permissions.rs:363-366`), so denying it here
        // would also strip a grant the same DocType gave another role.
        if !caps.submittable {
            rights.retain(|r| *r != Permission::Submit);
        }
        if !caps.allow_import {
            rights.retain(|r| *r != Permission::Import);
        }

        if rule.if_owner {
            rights.retain(|r| *r != Permission::Create);
            if rights.is_empty() {
                out.skipped.push(SkippedRule {
                    role: rule.role.clone(),
                    rights: Vec::new(),
                    reason: Untranslated::OwnerOnlyCreate,
                });
                continue;
            }
        } else if rights.is_empty() {
            out.skipped.push(SkippedRule {
                role: rule.role.clone(),
                rights: Vec::new(),
                reason: Untranslated::NoRights,
            });
            continue;
        }

        let resources = if rule.apply_to_all_doctypes {
            vec![Resource::new("*", "*")]
        } else {
            vec![
                Resource::new(doctype, "*"),
                Resource::new("doctype", doctype),
            ]
        };

        let principal = if rule.role == ALL_USER_ROLE {
            Principal::everyone()
        } else {
            Principal::role(&rule.role)
        };

        let statement = PolicyStatement::allow(vec![principal], rights, resources);
        if rule.if_owner {
            out.statements
                .push(statement.with_condition(OWNER_CONDITION, Value::Bool(true)));
        } else {
            out.statements.push(statement);
        }
    }

    out
}
