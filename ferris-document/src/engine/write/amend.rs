//! Amend: clone a submitted/cancelled document into a fresh draft.
//!
//! Frappe's amendment is not a copy: the engine assigns a new name from the
//! `NAME-1`, `NAME-2`, … chain, clears the engine-owned fields so the copy's
//! provenance is the amender (not the source owner), and re-runs the ordinary
//! insert lifecycle. The logic lives here, next to `rename` and `set_docstatus`,
//! so the REST route and the WASM `data` import delegate to one implementation
//! instead of each owning a subtly different copy.

use super::super::support::*;
use super::super::*;
use ferris_core::hooks::DocStatus;
use ferris_core::permissions::Permission;
use ferris_core::{Error, Result, ValidationError, Value};

impl DocumentEngine {
    /// Amend a cancelled document: read it with `Permission::Amend`, clone it
    /// into a new draft, and insert the copy through the normal lifecycle.
    ///
    /// Only a `Cancelled` source may be amended. Frappe's amendment flow is
    /// cancel → amend → submit; allowing a `Draft` or a `Submitted` source here
    /// let a caller duplicate live documents outside that flow, and the
    /// in-place `Cancelled → Draft` transition used to reopen the same row
    /// instead of creating the auditable copy (DOC-02).
    ///
    /// The copy clears `name` and every [`SYSTEM_FIELDS`] entry, so the engine
    /// stamps the new owner/creation/modified from the caller. A doctype that
    /// declares `amended_from` gets the source name recorded; one that does not
    /// never receives an undeclared key.
    ///
    /// # Errors
    ///
    /// The source read (including `Amend` authorization), a non-cancelled
    /// source, the amendment name allocation, or the insert lifecycle.
    // `ferris_core::Error` is the shared taxonomy; boxing it here would make
    // every caller match on a Box it never inspects.
    #[allow(clippy::result_large_err)]
    pub async fn amend(&self, ctx: &EngineContext<'_>, name: &str) -> Result<Value> {
        let stored = self.get_for(ctx, name, Permission::Amend).await?;
        let status = stored
            .get("docstatus")
            .and_then(Value::as_str)
            .map(docstatus_from_str)
            .unwrap_or(DocStatus::Draft);
        if status != DocStatus::Cancelled {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "only a cancelled document can be amended (got {status:?})"
            ))));
        }
        let mut doc = stored.as_object().cloned().unwrap_or_default();
        doc.remove("name");
        doc.insert("docstatus".into(), Value::from(0i64));
        for field in SYSTEM_FIELDS {
            doc.remove(field);
        }
        // A doctype may not allow amending, but it also may not have an
        // `amended_from` field; writing an undeclared key would store something
        // the form never shows.
        if ctx.meta.is_some_and(|meta| {
            meta.fields
                .iter()
                .any(|field| field.fieldname == "amended_from")
        }) {
            doc.insert("amended_from".into(), Value::from(name));
        }
        let amended_name = crate::naming::next_amend_name(&self.db, &ctx.doctype, name).await?;
        doc.insert("name".into(), Value::from(amended_name));
        // The amendment name is engine-chosen; bypass DOC-16's rejection of a
        // caller-supplied name for series/counter doctypes.
        self.insert_with_explicit_name(ctx, Value::Object(doc))
            .await
    }
}

// Tests live next to the code because the A7 patch scope lists only this file;
// the shared `engine::tests` modules are outside it.
#[cfg(test)]
mod tests {
    use super::*;

    /// DOC-02: only a cancelled source can be amended, and the copy records
    /// `amended_from`; a draft or submitted source is refused.
    #[tokio::test]
    async fn amend_requires_cancelled_and_records_provenance() {
        let db = ferris_db::memory_db().await.unwrap();
        crate::naming::NamingSeriesStore::ensure_table(&db)
            .await
            .unwrap();
        let engine = DocumentEngine::new(db);
        let meta = DoctypeMeta {
            name: "A7AmendDoc".to_string(),
            fields: vec![ferris_schema::FieldMeta {
                fieldname: "amended_from".to_string(),
                fieldtype: "Data".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let ctx = EngineContext::new("A7AmendDoc", "")
            .with_naming(NamingType::Prompt, None)
            .with_meta(&meta)
            .allow_anonymous(true);

        let created = engine
            .create(
                &ctx,
                ferris_core::value!({ "name": "A7-A-1", "title": "x" }),
            )
            .await
            .unwrap();
        let name = created.get("name").unwrap().as_str().unwrap().to_string();

        // A draft cannot be amended.
        let err = engine.amend(&ctx, &name).await.unwrap_err();
        assert!(err.to_string().contains("cancelled"), "{err:?}");

        engine
            .set_docstatus(&ctx, &name, DocStatus::Submitted)
            .await
            .unwrap();
        // A submitted source cannot be amended either.
        let err = engine.amend(&ctx, &name).await.unwrap_err();
        assert!(err.to_string().contains("cancelled"), "{err:?}");

        engine
            .set_docstatus(&ctx, &name, DocStatus::Cancelled)
            .await
            .unwrap();
        let copy = engine.amend(&ctx, &name).await.unwrap();
        assert_eq!(copy.get("name").and_then(|v| v.as_str()), Some("A7-A-1-1"));
        assert_eq!(
            copy.get("docstatus").and_then(|v| v.as_str()),
            Some("draft")
        );
        assert_eq!(
            copy.get("amended_from").and_then(|v| v.as_str()),
            Some(name.as_str())
        );

        // The amendment itself is a draft, so it cannot be amended again.
        let copy_name = copy.get("name").unwrap().as_str().unwrap().to_string();
        assert!(engine.amend(&ctx, &copy_name).await.is_err());
    }
}
