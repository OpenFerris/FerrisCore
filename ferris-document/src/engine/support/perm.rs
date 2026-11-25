use super::super::*;
use super::*;
use ferris_core::password::{PASSWORD_MASK, is_password_mask};
use ferris_core::permissions::Permission;
use ferris_core::permissions::{UserPermissionRule, UserPermissionRules, user_permissions_for};
use ferris_core::{Error, Result, ValidationError, Value};
use ferris_schema::FieldMeta;
use std::collections::HashMap;

/// Whether `ctx.user` may access fields declared at `permlevel` for `action`.
///
/// Delegates to [`ferris_core::permissions::has_permlevel_access`], which
/// encodes Frappe's `get_permlevel_access`: level 0 is the document itself and
/// was already authorized by [`check_permission`]; a higher level needs a
/// `DocPerm` row for one of the caller's roles that carries the right at that
/// level, and System Manager sees every level.
///
/// With no `DoctypeMeta` there are no declared levels, so nothing is hidden —
/// the same conservative no-op `validate` and `guard_update_after_submit` take.
fn field_level_allowed(ctx: &EngineContext<'_>, action: Permission, permlevel: u8) -> bool {
    let Some(meta) = ctx.meta else {
        return true;
    };
    // Anonymous: the document-level check already admitted (or not) this
    // request; there is no identity to attach a field level to, so only level 0
    // (the row itself) is reached.
    ctx.user.as_ref().map_or_else(
        || permlevel == 0,
        |user| {
            ferris_core::permissions::has_permlevel_access(
                &meta.permissions,
                user,
                action,
                permlevel,
            )
        },
    )
}

/// Whether `ctx.user` may see the real value of a **masked** field.
///
/// Frappe grants masking through a `mask` right on the field's `permlevel`
/// (`DocPerm.mask`). Ferris stores that right in the rule's
/// `custom_permissions` list, the same place a Permission Type lives, so the
/// effective `Custom DocPerm` merge and the Desk matrix already carry it.
/// System Manager keeps the field-level bypass every other right has.
fn field_mask_allowed(ctx: &EngineContext<'_>, field: &FieldMeta) -> bool {
    let Some(meta) = ctx.meta else {
        return true;
    };
    let Some(user) = ctx.user.as_ref() else {
        return false;
    };
    if user.has_role(ferris_core::permissions::SYSTEM_MANAGER_ROLE) {
        return true;
    }
    meta.permissions.iter().any(|rule| {
        rule.permlevel == field.permlevel
            && rule.grants_custom("mask")
            && (rule.role == ferris_core::permissions::ALL_USER_ROLE || user.has_role(&rule.role))
    })
}

/// A `Password` fieldtype: hashed on write, masked on read.
pub(in crate::engine) fn is_password_field(field: &FieldMeta) -> bool {
    field.fieldtype == "Password"
}

/// A `Secret` fieldtype: encrypted on write, masked on read, recoverable by a
/// trusted context through [`DocumentEngine::read_secret`].
pub(in crate::engine) fn is_secret_field(field: &FieldMeta) -> bool {
    field.fieldtype == "Secret"
}

/// Whether `ctx.user` may read this field.
///
/// Shared with the virtual-field path: a computed field is still a field, so a
/// virtual DocField at a restricted permlevel must not be inserted into a
/// document the caller reads. Without this check, redaction would remove the
/// stored value and the virtual provider would immediately put a computed value
/// back in its place.
pub(in crate::engine) fn field_read_allowed(ctx: &EngineContext<'_>, field: &FieldMeta) -> bool {
    field.permlevel == 0 || field_level_allowed(ctx, Permission::Read, field.permlevel)
}

/// Apply field-level **write** permissions to an incoming body.
///
/// Two things happen, mirroring Frappe's `apply_fieldlevel_write_permissions`
/// plus its credential handling:
///
/// * a field at a level the caller may not write is **dropped**, so the update
///   keeps the stored value and an insert stores nothing there; and
/// * a `Password` value that is the read mask means "unchanged" and is dropped,
///   while any other value is hashed before it can reach its column;
/// * a `Secret` value follows the same mask rule, but is **encrypted** (AES-GCM,
///   see `ferris_core::secret`) so a trusted runtime can recover it. Writing an
///   already-encrypted envelope is rejected: it would store a double-wrapped
///   value that every reader would then have to unwrap twice.
///
/// With no `DoctypeMeta` there is nothing declared, so the body is untouched —
/// the previous behaviour.
pub(in crate::engine) fn apply_field_permissions_write(
    ctx: &EngineContext<'_>,
    fields: &mut HashMap<String, Value>,
) -> Result<()> {
    // Mass assignment (DOC-08): keys the framework owns are refused before the
    // merge. Undeclared keys are rejected by the update path, which is where a
    // stored row exists for the metadata to describe; `insert` deliberately
    // keeps accepting them because the framework's own migrations write rows
    // through a partial `ctx.meta` (a legacy password written while the field
    // was undeclared, a runtime table with no declared fields). The keys that
    // matter — `idx`, `parent*`, `amended_from`, `workflow_state`,
    // `modified_by` — are refused on both paths, right here.
    reject_engine_field_writes(ctx, fields)?;
    // `User Permission`: a `Link` to a value outside the caller's rules is a
    // denial, before the field rules drop anything (a dropped field would
    // otherwise hide the attempt).
    reject_forbidden_user_permission_links(ctx, fields)?;
    let Some(meta) = ctx.meta else {
        return Ok(());
    };
    for field in &meta.fields {
        if field.permlevel > 0 && !field_level_allowed(ctx, Permission::Write, field.permlevel) {
            fields.remove(&field.fieldname);
            continue;
        }
        if is_secret_field(field) {
            match fields.get(&field.fieldname).and_then(Value::as_str) {
                None => {}
                Some(value) if value.is_empty() || is_password_mask(value) => {
                    fields.remove(&field.fieldname);
                }
                Some(plain) => {
                    if ferris_core::secret::is_secret_envelope(plain) {
                        return Err(Error::Validation(ValidationError::Invalid(format!(
                            "`{}` received an encrypted envelope; send the plaintext or the \
                             read mask",
                            field.fieldname
                        ))));
                    }
                    let scope = ferris_core::secret::SecretScope {
                        doctype: &ctx.doctype,
                        field: &field.fieldname,
                    };
                    let envelope = ferris_core::secret::encrypt_secret(scope, plain)?;
                    fields.insert(field.fieldname.clone(), Value::from(envelope));
                }
            }
            continue;
        }
        if !is_password_field(field) {
            continue;
        }
        match fields.get(&field.fieldname).and_then(Value::as_str) {
            None => {}
            Some(value) if value.is_empty() || is_password_mask(value) => {
                fields.remove(&field.fieldname);
            }
            Some(plain) => {
                let hashed = ferris_core::password::hash_password(plain)?;
                fields.insert(field.fieldname.clone(), Value::from(hashed));
            }
        }
    }
    Ok(())
}

/// Apply field-level **read** permissions to a document about to be returned.
///
/// A field at a level the caller may not read is removed, and a `Password` value
/// is replaced by [`PASSWORD_MASK`] so a hash never travels through a read
/// response. The mask applies to **every** HTTP caller, System Manager included:
/// a role is not a reason to hand out an Argon2 hash. The authentication store
/// is the only reader that opts out, through
/// [`EngineContext::reveal_password_hash`], and that context is never built from
/// a request.
///
/// A `Secret` value is masked unconditionally, even for a context with
/// [`EngineContext::reveal_secrets`]: that flag only unlocks
/// [`DocumentEngine::read_secret`], which returns the plaintext out of band
/// instead of putting it in a document that derives `Debug`.
///
/// The lifecycle reads (`update`/`delete`/`set_docstatus`) deliberately do **not**
/// go through this: they operate on stored state and run hooks that expect it.
pub(in crate::engine) fn redact_read(ctx: &EngineContext<'_>, value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    let Some(meta) = ctx.meta else {
        return Value::Object(map);
    };
    for field in &meta.fields {
        if field.permlevel > 0 && !field_level_allowed(ctx, Permission::Read, field.permlevel) {
            map.remove(&field.fieldname);
            continue;
        }
        if is_secret_field(field) && map.contains_key(&field.fieldname) {
            map.insert(
                field.fieldname.clone(),
                Value::from(ferris_core::secret::SECRET_MASK),
            );
            continue;
        }
        if is_password_field(field)
            && !ctx.reveal_password_hash
            && map.contains_key(&field.fieldname)
        {
            map.insert(field.fieldname.clone(), Value::from(PASSWORD_MASK));
            continue;
        }
        if field.mask && map.contains_key(&field.fieldname) && !field_mask_allowed(ctx, field) {
            map.insert(field.fieldname.clone(), Value::from(PASSWORD_MASK));
        }
    }
    Value::Object(map)
}

/// The owner a permission check should be evaluated against.
///
/// A Single `DocType`'s one document is **global**, not owned by anyone: it is
/// authorized purely by role rules, so the owner shortcut is disabled for it
/// (`None`). Every other `DocType` keeps the stored column as the owner, which
/// is what the owner rule and `if_owner` statements rely on.
pub(in crate::engine) fn authz_owner<'a>(
    ctx: &EngineContext<'_>,
    stored_owner: Option<&'a str>,
) -> Option<&'a str> {
    if is_single(ctx) { None } else { stored_owner }
}

/// The `User Permission` rules that say something about `ctx.doctype`, grouped
/// by the `DocType` they allow.
///
/// Grouping is what keeps the semantics right: several rows on the same allowed
/// `DocType` are *alternatives* (`Customer A` or `Customer B`), while rows on
/// different `DocType`s are *conjunctions* (the document must link to an
/// allowed `Customer` **and** an allowed `Territory`). That is Frappe's
/// `user_permissions` map, one list of values per allowed `DocType`.
type RuleGroups<'a> = Vec<(String, Vec<&'a UserPermissionRule>)>;

fn user_permission_groups<'a>(
    ctx: &EngineContext<'_>,
    rules: &'a UserPermissionRules,
) -> RuleGroups<'a> {
    let mut groups: RuleGroups<'a> = Vec::new();
    for rule in rules.iter().filter(|rule| rule.applies_to(&ctx.doctype)) {
        match groups.iter_mut().find(|(allow, _)| *allow == rule.allow) {
            Some((_, group)) => group.push(rule),
            None => groups.push((rule.allow.clone(), vec![rule])),
        }
    }
    groups
}

/// The installed rules of the acting user, or nothing when unrestricted.
///
/// One lookup per call, served from the installed in-memory snapshot — never a
/// query per row. A `System Manager` is unrestricted, the same bypass every
/// other engine decision takes.
fn acting_user_permissions(ctx: &EngineContext<'_>) -> Option<UserPermissionRules> {
    let user = ctx.user.as_ref()?;
    if user.has_role(ferris_core::permissions::SYSTEM_MANAGER_ROLE) {
        return None;
    }
    let rules = user_permissions_for(&user.id);
    if rules.is_empty() { None } else { Some(rules) }
}

/// Link fields of `ctx.doctype` that point at `allow`.
///
/// `ignore_user_permissions` is Frappe's per-field opt-out and is honoured: a
/// field that declares it is not narrowed by a `User Permission`.
fn user_permission_fields<'a>(ctx: &'a EngineContext<'_>, allow: &str) -> Vec<&'a FieldMeta> {
    let Some(meta) = ctx.meta else {
        return Vec::new();
    };
    meta.fields
        .iter()
        .filter(|field| {
            !field.ignore_user_permissions
                && field.fieldtype == "Link"
                && field.options.as_deref() == Some(allow)
        })
        .collect()
}

/// Whether one group of rules admits `document`.
///
/// * The document **is** the allowed `DocType`: its own name is the value.
/// * Otherwise every `Link` field to the allowed `DocType` must hold an allowed
///   value. A `DocType` with no such field is not reached by the rule at all —
///   the conservative `PerRow` scope from `read_scope` only costs a check here,
///   it does not deny rows the rule cannot speak about.
fn user_permission_group_allows(
    ctx: &EngineContext<'_>,
    document: &Value,
    allow: &str,
    group: &[&UserPermissionRule],
) -> bool {
    if allow == ctx.doctype {
        let name = document
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return group.iter().any(|rule| rule.allows(name));
    }
    user_permission_fields(ctx, allow).into_iter().all(|field| {
        document
            .get(&field.fieldname)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty() && group.iter().any(|rule| rule.allows(value)))
    })
}

/// Whether the acting user's `User Permission` rules admit `document`.
///
/// The engine's per-row predicate. The role decision stays where it is; this is
/// the value restriction layered on top of it, and it needs the document
/// because the restriction is about the row's `Link` values.
pub(in crate::engine) fn user_permission_allows(ctx: &EngineContext<'_>, document: &Value) -> bool {
    let Some(rules) = acting_user_permissions(ctx) else {
        return true;
    };
    user_permission_groups(ctx, &rules)
        .iter()
        .all(|(allow, group)| user_permission_group_allows(ctx, document, allow, group))
}

/// [`user_permission_allows`] as an error, for any action.
///
/// The read paths used to be the only callers; `update` and `delete` authorised
/// against the role rules alone, so a user limited to `Customer A` could still
/// rewrite or trash a `Customer B` row it owned. Every action that reads a
/// stored document now runs this against that document.
pub(in crate::engine) fn enforce_user_permission(
    ctx: &EngineContext<'_>,
    document: &Value,
    action: Permission,
) -> Result<()> {
    if user_permission_allows(ctx, document) {
        return Ok(());
    }
    let name = document
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Err(Error::Authz(ferris_core::AuthzError::PermissionDenied {
        action: action.as_str().to_string(),
        resource: format!("{} {name}", ctx.doctype),
    }))
}

/// [`enforce_user_permission`] for a read.
pub(in crate::engine) fn enforce_user_permission_read(
    ctx: &EngineContext<'_>,
    document: &Value,
) -> Result<()> {
    enforce_user_permission(ctx, document, Permission::Read)
}

/// Reject a write whose `Link` fields name a value the caller may not reach.
///
/// The write counterpart of [`user_permission_allows`], run on the incoming
/// body (insert and update) by [`apply_field_permissions_write`]. A value
/// outside the user's `User Permission` is a `PermissionDenied` — the same
/// answer Frappe's `validate_user_permissions` gives — rather than a silent
/// no-op, because silently storing a link the caller cannot read would be the
/// exact confusion this capability is meant to remove.
///
/// Only fields the body carries are checked: a stored row was validated when it
/// was written, and re-reading it belongs to the read path.
pub(in crate::engine) fn reject_forbidden_user_permission_links(
    ctx: &EngineContext<'_>,
    fields: &HashMap<String, Value>,
) -> Result<()> {
    let Some(rules) = acting_user_permissions(ctx) else {
        return Ok(());
    };
    for (allow, group) in user_permission_groups(ctx, &rules) {
        // The document's own identity, when the body names it (an explicit
        // `name` under `Prompt`/`Field` naming). A minted name never reaches
        // here, which matches Frappe: the link fields are what is validated.
        if allow == ctx.doctype {
            if let Some(name) = fields.get("name").and_then(Value::as_str)
                && !name.is_empty()
                && !group.iter().any(|rule| rule.allows(name))
            {
                return Err(forbidden_value(ctx, "name", name));
            }
            continue;
        }
        for field in user_permission_fields(ctx, &allow) {
            let Some(value) = fields.get(&field.fieldname).and_then(Value::as_str) else {
                continue;
            };
            if value.is_empty() || group.iter().any(|rule| rule.allows(value)) {
                continue;
            }
            return Err(forbidden_value(ctx, &field.fieldname, value));
        }
    }
    Ok(())
}

/// The typed denial for a `Link` value outside the caller's `User Permission`.
fn forbidden_value(ctx: &EngineContext<'_>, field: &str, value: &str) -> Error {
    Error::Authz(ferris_core::AuthzError::PermissionDenied {
        action: Permission::Write.as_str().to_string(),
        resource: format!("{} {field}={value}", ctx.doctype),
    })
}

/// Authorize `action` on `name`.
///
/// `trusted_owner` **must** come from stored state (the row read from the
/// database), never from a caller-supplied payload.
///
/// **Why this is not just a convention.** This function used to take
/// `fields: &HashMap<String, Value>` and read `owner` out of it. On the update
/// path that map was the *merged* document — the stored row with the client's
/// body applied on top — so `PUT /api/resource/<DocType>/<name>` with
/// `{"owner": "<me>"}` made `user.id == resource.owner` true, and the owner
/// shortcut in `ferris_core::permissions::check_permission` granted `Write` on a
/// document the caller did not own. The takeover was then persisted, so it was
/// permanent and left no trace.
///
/// Taking the owner as a separate parameter removes any path from a request body
/// to the authorization decision: it is no longer *possible* to pass
/// attacker-controlled data here by accident. Frappe is equivalent and goes
/// further — it evaluates `has_permission` against the stored document and
/// overwrites `owner` from the session on every save
/// (`frappe/model/document.py:1055-1070`).
pub(in crate::engine) fn check_permission(
    ctx: &EngineContext<'_>,
    name: &str,
    trusted_owner: Option<&str>,
    action: Permission,
) -> Result<()> {
    let Some(user) = ctx.user.as_ref() else {
        // Default-deny: anonymous operations fail closed unless the context
        // explicitly opts in with `allow_anonymous`.
        return if ctx.allow_anonymous {
            Ok(())
        } else {
            Err(Error::Authz(ferris_core::AuthzError::PermissionDenied {
                action: action.as_str().to_string(),
                resource: format!("{} {name}", ctx.doctype),
            }))
        };
    };
    let resource = ferris_core::permissions::Resource::new(&ctx.doctype, name)
        .with_owner(trusted_owner.unwrap_or(""));
    if ferris_core::permissions::check_permission(user, &resource, action) {
        Ok(())
    } else {
        Err(Error::Authz(ferris_core::AuthzError::PermissionDenied {
            action: action.as_str().to_string(),
            resource: format!("{} {name}", ctx.doctype),
        }))
    }
}

/// Fields the engine owns. A request body may not set them to a different value.
///
/// Mirrors `Document.set_user_and_timestamp` in Frappe
/// (`frappe/model/document.py:1055-1070`): `modified` and `modified_by` are
/// stamped from the session on **every** save, while `creation` and `owner` are
/// set once at insert and never moved by a later save.
///
/// The structural keys (`idx`, `parent`, `parenttype`, `parentfield`,
/// `amended_from`) and the workflow state are in the same list because they are
/// the framework's own bookkeeping, not document data: a body that carries them
/// is trying to pick its row order, its parent link, its amendment lineage or
/// its workflow position. `workflow_state` moves only through a declared
/// transition (`engine::write::workflow`), which is why the engine, not the
/// client, writes it.
pub const SYSTEM_FIELDS: [&str; 10] = [
    "owner",
    "creation",
    "modified",
    "modified_by",
    "workflow_state",
    "idx",
    "parent",
    "parenttype",
    "parentfield",
    "amended_from",
];

/// System fields that carry no caller meaning at all, whatever the value.
///
/// `owner`/`creation`/`modified`/`modified_by`/`docstatus`/`amended_from` accept
/// the value the engine already stores (a read-modify-write round trip is
/// legitimate); these carry no such meaning and are refused outright.
/// `workflow_state` is excluded here because the update path validates and then
/// strips it before this check, and an insert must not name one.
const STRUCTURAL_FIELDS: [&str; 4] = ["idx", "parent", "parenttype", "parentfield"];

/// Reject the keys that are never document data, and refute a caller-chosen
/// `modified_by`.
///
/// Runs on both insert and update, before the field-level rules, so an
/// undeclared key never reaches a column. `modified_by` is accepted only when it
/// equals the acting session's id (the value the engine is about to stamp);
/// with no session it is dropped, because keeping a caller-chosen name there
/// would attribute the write to somebody who did not make it.
pub(in crate::engine) fn reject_engine_field_writes(
    ctx: &EngineContext<'_>,
    fields: &mut HashMap<String, Value>,
) -> Result<()> {
    for key in STRUCTURAL_FIELDS {
        // A DocType may *declare* one of these: `Custom DocPerm` declares
        // `parent` (the DocType its row belongs to) because it is shaped like a
        // child table, and `parent` there is the row's real link. A declared key
        // is ordinary document data — the update path still pins it to the
        // stored value — while an undeclared one has no meaning and is
        // mass assignment.
        if ctx
            .meta
            .is_some_and(|meta| meta.fields.iter().any(|field| field.fieldname == *key))
        {
            continue;
        }
        if fields.contains_key(key) {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "`{key}` is set by the engine and cannot be supplied by the client"
            ))));
        }
    }
    match ctx.user.as_ref() {
        Some(user) => {
            reject_system_field_overrides(fields, &[("modified_by", user.id.as_str())])?;
        }
        None => {
            fields.remove("modified_by");
        }
    }
    Ok(())
}

/// Reject a key the effective metadata does not declare.
///
/// Frappe's document model only accepts declared fields: an unknown key either
/// belongs to a DocType the caller is confusing with this one or is a probe for
/// a column the code trusts. With no `DoctypeMeta` (unregistered DocTypes, the
/// runtime side tables) there is nothing declared, so the previous behaviour —
/// accept whatever the caller sent — is kept, and so it is for a `DocType` that
/// declares no fields at all: a metadata with an empty field list cannot tell
/// "undeclared" from "not described here".
///
/// Engine-owned keys are exempt: they are checked by their own rule, so a
/// round-tripped document (`owner`, `modified`, `docstatus`, `name`, …) still
/// passes.
pub(in crate::engine) fn reject_undeclared_fields(
    ctx: &EngineContext<'_>,
    fields: &HashMap<String, Value>,
) -> Result<()> {
    let Some(meta) = ctx.meta.filter(|meta| !meta.fields.is_empty()) else {
        return Ok(());
    };
    for key in fields.keys() {
        if is_engine_field(key) || key == "doctype" {
            continue;
        }
        if meta.fields.iter().any(|field| field.fieldname == *key) {
            continue;
        }
        return Err(Error::Validation(ValidationError::Invalid(format!(
            "`{key}` is not a field of `{}`",
            ctx.doctype
        ))));
    }
    Ok(())
}

/// Reject a body that tries to set a system field to a value other than the one
/// the engine is about to apply.
///
/// Frappe silently overwrites. Ferris returns `400` instead, because here the
/// caller is an HTTP client rather than an in-process importer: silently
/// dropping the field would let a caller believe it had transferred ownership
/// when nothing happened, which is exactly the confusion this bug produced.
pub(in crate::engine) fn reject_system_field_overrides(
    fields: &HashMap<String, Value>,
    resolved: &[(&str, &str)],
) -> Result<()> {
    for (key, expected) in resolved {
        match field_str(fields, key) {
            None => {}
            Some(actual) if actual == *expected => {}
            Some(actual) => {
                return Err(Error::Validation(ValidationError::Invalid(format!(
                    "`{key}` is set by the engine and cannot be supplied by the client \
                     (got {actual:?})"
                ))));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferris_core::permissions::{
        UserPermissionRules, UserPermissionSource, clear_user_permission_source,
        set_user_permission_source,
    };
    use ferris_schema::DoctypeMeta;
    use std::sync::Arc;

    /// A source with a fixed rule list; the port is process-wide, so these
    /// tests serialise and clear it on drop.
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

    fn customer_meta() -> DoctypeMeta {
        DoctypeMeta {
            name: "D1Customer".to_string(),
            fields: vec![FieldMeta::data("title")],
            ..Default::default()
        }
    }

    fn order_meta() -> DoctypeMeta {
        DoctypeMeta {
            name: "D1Order".to_string(),
            fields: vec![
                FieldMeta::link("customer", "D1Customer"),
                FieldMeta::link("bill_to", "D1Customer"),
            ],
            ..Default::default()
        }
    }

    fn ada() -> ferris_core::User {
        let mut user = ferris_core::User::system_user("ada@local".to_string(), "Ada".to_string());
        user.id = "ada@local".to_string();
        user.roles = vec!["D1 Reader".to_string()];
        user
    }

    fn rule_allow_a() -> UserPermissionRule {
        UserPermissionRule::new("ada@local", "D1Customer", "CUST-A")
            .with_apply_to_all_doctypes(true)
    }

    /// A user restricted to `CUST-A` does not read `CUST-B`: the row's own
    /// name is the value when the document is the allowed `DocType`.
    #[test]
    fn a_restricted_user_cannot_read_the_other_value() {
        let _installed = install(vec![rule_allow_a()]);
        let meta = customer_meta();
        let user = ada();
        let ctx = EngineContext::new("D1Customer", "")
            .with_meta(&meta)
            .with_user(&user);
        assert!(user_permission_allows(
            &ctx,
            &ferris_core::value!({ "name": "CUST-A" })
        ));
        assert!(!user_permission_allows(
            &ctx,
            &ferris_core::value!({ "name": "CUST-B" })
        ));
        let err = enforce_user_permission_read(&ctx, &ferris_core::value!({ "name": "CUST-B" }))
            .expect_err("CUST-B must be denied");
        assert!(matches!(err, Error::Authz(_)), "{err:?}");
    }

    /// A `Link` to `CUST-B` is rejected on write, and one to `CUST-A` passes.
    #[test]
    fn a_link_to_a_forbidden_value_is_rejected_on_write() {
        let _installed = install(vec![rule_allow_a()]);
        let meta = order_meta();
        let user = ada();
        let ctx = EngineContext::new("D1Order", "")
            .with_meta(&meta)
            .with_user(&user);

        let fields = to_fields(&ferris_core::value!({ "customer": "CUST-A" })).unwrap();
        assert!(reject_forbidden_user_permission_links(&ctx, &fields).is_ok());
        let fields = to_fields(&ferris_core::value!({ "customer": "CUST-B" })).unwrap();
        let err = reject_forbidden_user_permission_links(&ctx, &fields)
            .expect_err("a link outside the rule must be rejected");
        assert!(matches!(err, Error::Authz(_)), "{err:?}");
    }

    /// With no source installed the previous behaviour is kept exactly: no rule,
    /// no narrowing, on read and on write.
    #[test]
    fn without_rules_nothing_is_narrowed() {
        let _installed = install(Vec::new());
        let meta = order_meta();
        let user = ada();
        let ctx = EngineContext::new("D1Order", "")
            .with_meta(&meta)
            .with_user(&user);
        let fields = to_fields(&ferris_core::value!({ "customer": "CUST-B" })).unwrap();
        assert!(reject_forbidden_user_permission_links(&ctx, &fields).is_ok());
        assert!(user_permission_allows(
            &ctx,
            &ferris_core::value!({ "customer": "CUST-B" })
        ));
    }

    /// `apply_to_all_doctypes` reaches a `DocType` that links to the allowed one.
    #[test]
    fn apply_to_all_doctypes_narrows_every_linking_doctype() {
        let _installed = install(vec![rule_allow_a()]);
        let meta = order_meta();
        let user = ada();
        let ctx = EngineContext::new("D1Order", "")
            .with_meta(&meta)
            .with_user(&user);
        assert!(user_permission_allows(
            &ctx,
            &ferris_core::value!({ "customer": "CUST-A", "bill_to": "CUST-A" })
        ));
        assert!(!user_permission_allows(
            &ctx,
            &ferris_core::value!({ "customer": "CUST-A", "bill_to": "CUST-B" })
        ));
        // A DocType with no link to the allowed DocType is not reached.
        let todo = DoctypeMeta {
            name: "D1Todo".to_string(),
            fields: vec![FieldMeta::data("title")],
            ..Default::default()
        };
        let todo_ctx = EngineContext::new("D1Todo", "")
            .with_meta(&todo)
            .with_user(&user);
        assert!(user_permission_allows(
            &todo_ctx,
            &ferris_core::value!({ "name": "TODO-1", "title": "x" })
        ));
    }

    /// `hide_descendants` is part of the allowed value, so a child node passes
    /// the same check an exact match does.
    #[test]
    fn hide_descendants_admits_a_resolved_child() {
        let _installed = install(vec![
            UserPermissionRule::new("ada@local", "D1Customer", "CUST-A")
                .with_hide_descendants(true)
                .with_descendants(vec!["CUST-A-CHILD".to_string()]),
        ]);
        let meta = customer_meta();
        let user = ada();
        let ctx = EngineContext::new("D1Customer", "")
            .with_meta(&meta)
            .with_user(&user);
        assert!(user_permission_allows(
            &ctx,
            &ferris_core::value!({ "name": "CUST-A-CHILD" })
        ));
        assert!(!user_permission_allows(
            &ctx,
            &ferris_core::value!({ "name": "CUST-B" })
        ));
    }

    /// A `System Manager` is unrestricted even when a rule names them.
    #[test]
    fn system_manager_is_never_narrowed() {
        let _installed = install(vec![
            UserPermissionRule::new("root@local", "D1Customer", "CUST-A")
                .with_apply_to_all_doctypes(true),
        ]);
        let meta = customer_meta();
        let mut user = ferris_core::User::system_user("root@local".to_string(), "Root".to_string());
        user.id = "root@local".to_string();
        user.roles = vec![ferris_core::permissions::SYSTEM_MANAGER_ROLE.to_string()];
        let ctx = EngineContext::new("D1Customer", "")
            .with_meta(&meta)
            .with_user(&user);
        assert!(user_permission_allows(
            &ctx,
            &ferris_core::value!({ "name": "CUST-B" })
        ));
    }
}
