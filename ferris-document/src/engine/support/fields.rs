use super::super::*;
use super::*;
use ferris_core::hooks::DocStatus;
use ferris_core::permissions::Permission;
use ferris_core::{Error, Result, ValidationError, Value};
use ferris_db::Stmt;
use std::collections::HashMap;

pub(in crate::engine) fn set_docstatus_value(mut document: Value, status: DocStatus) -> Value {
    if let Value::Object(ref mut map) = document {
        map.insert("docstatus".to_string(), Value::from(docstatus_str(status)));
    }
    document
}

pub(in crate::engine) fn field_str(fields: &HashMap<String, Value>, key: &str) -> Option<String> {
    fields
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Format a Unix timestamp as RFC 3339.
///
/// DOC13: this used to fall back to `chrono::Utc::now()` when
/// `from_timestamp` failed, which quietly stamped a document with the wrong
/// time instead of the timestamp it was given. The caller always passes
/// [`chrono::Utc::now`]`().timestamp()`, so the conversion does not fail today —
/// but a fallible API that masks its failure is exactly the "no silent success"
/// shape `AGENTS.md` §4 forbids, so the error is propagated instead.
pub(in crate::engine) fn rfc3339(ts: i64) -> Result<String> {
    match chrono::DateTime::from_timestamp(ts, 0) {
        Some(dt) => Ok(dt.to_rfc3339()),
        None => Err(Error::Validation(ValidationError::Invalid(format!(
            "timestamp {ts} is outside the supported range"
        )))),
    }
}

/// [`DocumentEngine::read_row`] through an explicit [`WriteSink`], so `apply`
/// reads inside its open transaction. The table must already be ensured by the
/// caller: `apply` creates every table before it begins, and a DDL statement
/// must never run inside the transaction.
/// Write the system and declared columns through `sink`.
///
/// `owner` is only overwritten when the document carries one, so an anonymous or
/// system write cannot blank the owner a previous write attributed.
///
/// Sink-agnostic so a caller inside an open transaction (`create_bulk`, `apply`,
/// `set_docstatus`) persists on that transaction's connection instead of
/// sneaking a second connection in. The columns are the only storage: there is
/// no `data` blob (ADR 0001). The caller passes the shape it already resolved,
/// so the write cannot disagree with the read it was built from and no metadata
/// query runs while the transaction holds the connection.
pub(in crate::engine) async fn persist_written(
    sink: &mut WriteSink<'_>,
    table: &str,
    doctype: &str,
    name: &str,
    document: &Value,
    shape: &DocShape,
    index_search: bool,
) -> Result<()> {
    let docstatus = document
        .get("docstatus")
        .and_then(|v| v.as_str())
        .map(docstatus_from_str)
        .unwrap_or(DocStatus::Draft);
    let owner = document.get("owner").and_then(|v| v.as_str());
    let typed = &shape.columns;
    let extra = update_set(typed, sink.dialect())?;
    let mut params = vec![
        Value::from(owner),
        Value::from(i64::from(docstatus as i16)),
        Value::from(chrono::Utc::now().timestamp()),
        Value::from(document.get("modified_by").and_then(Value::as_str)),
        Value::from(document.get("amended_from").and_then(Value::as_str)),
        Value::from(document.get("workflow_state").and_then(Value::as_str)),
    ];
    params.extend(column_values(typed, document));
    params.push(Value::from(name));
    sink.execute(&Stmt::new(
        format!(
            "UPDATE {} SET owner = COALESCE(?, owner), docstatus = ?, modified = ?, \
             modified_by = ?, amended_from = ?, workflow_state = ?{extra} \
             WHERE name = ?",
            q(sink.dialect(), table)?
        ),
        params,
    ))
    .await?;
    // The child arrays travel with the document; replace their real rows in the
    // same transaction as the parent.
    write_children(sink, shape, doctype, name, document).await?;
    if index_search && let Err(error) = search::index(sink, doctype, document, shape).await {
        // Best effort, like `insert_row`: the row is saved whatever the index
        // does, and the Rust scan still answers a search for it.
        tracing::warn!(
            %error,
            doctype,
            name,
            "document saved but not indexed for full-text search"
        );
    }
    Ok(())
}

pub(in crate::engine) fn project(value: Value, fields: &[String]) -> Value {
    if let Value::Object(map) = value {
        let mut out = ferris_core::Map::new();
        for field in fields {
            if let Some(v) = map.get(field) {
                out.insert(field.clone(), v.clone());
            }
        }
        Value::Object(out)
    } else {
        value
    }
}

/// Whether `ctx.user` may `Read` this row. `owner` must come from the `owner`
/// column, never from the document body.
///
/// Same default-deny contract as [`check_permission`]: an anonymous caller is
/// refused unless the context explicitly opted in with `allow_anonymous`. This
/// is the predicate form used by `list()`, where a refusal drops the row
/// instead of failing the whole query — one unreadable row must not hide the
/// rest of the page.
pub(in crate::engine) fn can_read(
    ctx: &EngineContext<'_>,
    name: &str,
    owner: Option<&str>,
) -> Result<()> {
    let Some(user) = ctx.user.as_ref() else {
        return if ctx.allow_anonymous {
            Ok(())
        } else {
            Err(Error::Authz(ferris_core::AuthzError::PermissionDenied {
                action: Permission::Read.as_str().to_string(),
                resource: format!("{} {name}", ctx.doctype),
            }))
        };
    };
    let resource = ferris_core::permissions::Resource::new(&ctx.doctype, name)
        .with_owner(authz_owner(ctx, owner).unwrap_or(""));
    if ferris_core::permissions::check_permission(user, &resource, Permission::Read) {
        Ok(())
    } else {
        Err(Error::Authz(ferris_core::AuthzError::PermissionDenied {
            action: Permission::Read.as_str().to_string(),
            resource: format!("{} {name}", ctx.doctype),
        }))
    }
}

pub(in crate::engine) fn validate(meta: Option<&DoctypeMeta>, value: &Value) -> Result<()> {
    let Some(meta) = meta else {
        return Ok(());
    };
    for field in &meta.fields {
        if field.fieldname == "name" {
            continue;
        }
        let field_value = value.get(&field.fieldname).cloned().unwrap_or(Value::Null);
        crate::validation::validate_field(&field.fieldname, &field_value, field)?;
    }
    Ok(())
}

/// Whether the context's `DocType` is a Single `DocType`.
///
/// `ctx.meta` is `None` for unregistered `DocTypes` (test and runtime tables);
/// those are never singles, so the conservative answer is `false`.
pub(in crate::engine) fn is_single(ctx: &EngineContext<'_>) -> bool {
    ctx.meta.is_some_and(|m| m.is_single)
}
/// §C5: post-submit field lock — Frappe's `_validate_update_after_submit`
/// (`frappe/model/document.py:1485-1498`, `base_document.py:1363-1398`).
///
/// Once a document is **submitted**, a writer may only change fields the
/// doctype marks `allow_on_submit`; a **cancelled** document is fully locked
/// (amendment is the copy-and-`create` flow, not an in-place edit). `docstatus`
/// itself is pinned separately as an engine field, so it never reaches this
/// comparison as a "change". When no doctype metadata is available there is no
/// way to prove a field is `allow_on_submit`, so the conservative aggregate
/// default is to reject every change to a post-submit row.
pub(in crate::engine) fn guard_update_after_submit(
    meta: Option<&DoctypeMeta>,
    stored: &HashMap<String, Value>,
    merged: &HashMap<String, Value>,
) -> Result<()> {
    let status = docstatus_from_str(field_str(stored, "docstatus").as_deref().unwrap_or("draft"));
    if status == DocStatus::Draft {
        return Ok(());
    }
    for (key, value) in merged {
        if is_engine_field(key) {
            continue;
        }
        // Unchanged fields never fault; only a real edit is compared.
        if stored.get(key) == Some(value) {
            continue;
        }
        match status {
            DocStatus::Cancelled => {
                return Err(Error::Validation(ValidationError::Invalid(format!(
                    "`{key}` cannot be changed on a cancelled document"
                ))));
            }
            DocStatus::Submitted => {
                let allow_on_submit = meta.is_some_and(|m| {
                    m.fields
                        .iter()
                        .any(|f| f.fieldname == *key && f.allow_on_submit)
                });
                if !allow_on_submit {
                    return Err(Error::Validation(ValidationError::Invalid(format!(
                        "`{key}` cannot be changed on a submitted document \
                         (allow_on_submit = false)"
                    ))));
                }
            }
            DocStatus::Draft => unreachable!(),
        }
    }
    Ok(())
}

/// Fields the engine stamps itself on every save; they never count as a
/// document edit and are excluded from the post-submit lock.
///
/// `pub(crate)` and re-exported from [`super`] so the activity feed
/// ([`crate::activity`]) computes "which fields changed" with the *same* rule the
/// engine uses to decide what is an edit — a second copy would drift and start
/// reporting `modified` as a user change.
pub(crate) fn is_engine_field(key: &str) -> bool {
    SYSTEM_FIELDS.contains(&key) || matches!(key, "name" | "docstatus")
}
