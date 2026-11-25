//! Recoverable `Secret` fields: the one path that returns plaintext.
//!
//! Ordinary reads mask a `Secret` field exactly like a `Password` (see
//! [`super::support::redact_read`]), because a document is a [`Value`] that
//! derives `Debug` and travels over HTTP. A connector, on the other hand, needs
//! the original value to sign a webhook body or exchange an OAuth code.
//!
//! That plaintext is therefore **never** part of a document. It is returned by
//! the methods in this module as a [`SecretValue`] — redacted `Debug`, no
//! `Display`, no `Serialize` — to an [`EngineContext`] that explicitly opted in
//! with [`EngineContext::reveal_secrets`]. Requiring the flag keeps every
//! plaintext reader greppable (`read_secret`), and makes a context built from a
//! request fail closed instead of quietly decrypting.
//!
//! Authorization is the same one a normal read goes through: the DocType's role
//! rules plus, for the value itself, the field's `permlevel`
//! ([`field_read_allowed`]). A right to the document is not automatically a
//! right to its most restricted field.

use super::support::*;
use super::*;
use ferris_core::SecretError;
use ferris_core::permissions::Permission;
use ferris_core::secret::{SecretScope, SecretValue};
use std::collections::HashMap;

impl DocumentEngine {
    /// Decrypt one `Secret` field of the named document.
    ///
    /// Returns `Ok(None)` when the field is absent or empty (an unset webhook
    /// secret is not an error), the typed [`SecretError`] when the stored value
    /// cannot be recovered, and the plaintext otherwise.
    ///
    /// # Errors
    ///
    /// [`SecretError::RevealNotPermitted`] when `ctx` did not call
    /// [`EngineContext::reveal_secrets`], [`SecretError::NotSecretField`] when
    /// the field is not declared `Secret`, the engine's authorization error when
    /// the caller may not read the document or the field level, and the
    /// decryption failures documented on [`ferris_core::secret::decrypt_secret`].
    pub async fn read_secret(
        &self,
        ctx: &EngineContext<'_>,
        name: &str,
        field: &str,
    ) -> Result<Option<SecretValue>> {
        ensure_reveal_allowed(ctx)?;
        ensure_secret_field(ctx, field)?;
        let stored = self.authorized_secret_row(ctx, name).await?;
        decrypt_field(ctx, &stored, field)
    }

    /// Decrypt several `Secret` fields of one document with one authorization.
    ///
    /// Absent/empty fields are omitted from the map. One unrecoverable value
    /// fails the whole call: a caller that asked for `client_secret` and
    /// `refresh_token` must not proceed with half a configuration believing it
    /// is complete.
    ///
    /// # Errors
    ///
    /// The same failures as [`Self::read_secret`].
    pub async fn read_secrets(
        &self,
        ctx: &EngineContext<'_>,
        name: &str,
        fields: &[&str],
    ) -> Result<HashMap<String, SecretValue>> {
        ensure_reveal_allowed(ctx)?;
        for field in fields {
            ensure_secret_field(ctx, field)?;
        }
        let stored = self.authorized_secret_row(ctx, name).await?;
        let mut secrets = HashMap::with_capacity(fields.len());
        for field in fields {
            if let Some(value) = decrypt_field(ctx, &stored, field)? {
                secrets.insert((*field).to_string(), value);
            }
        }
        Ok(secrets)
    }

    /// Decrypt `Secret` fields of the context's Single document.
    ///
    /// A Single's row is named after its DocType, so the caller does not repeat
    /// it. The row must already exist ([`Self::get_single`] materialises it).
    ///
    /// # Errors
    ///
    /// [`ValidationError::Invalid`] when the context's DocType is not a Single,
    /// plus the failures of [`Self::read_secrets`].
    pub async fn read_single_secrets(
        &self,
        ctx: &EngineContext<'_>,
        fields: &[&str],
    ) -> Result<HashMap<String, SecretValue>> {
        if !is_single(ctx) {
            return Err(Error::Validation(ValidationError::Invalid(format!(
                "`{}` is not a Single DocType",
                ctx.doctype
            ))));
        }
        self.read_secrets(ctx, &ctx.doctype, fields).await
    }

    /// The stored row, authorized for `Read` exactly as [`Self::get_for`] does.
    ///
    /// Duplicating the three-step `read_row`/`authorize` sequence instead of
    /// calling `get_for` is deliberate: `get_for` returns the *redacted*
    /// document, in which the ciphertext has already been replaced by the mask.
    async fn authorized_secret_row(&self, ctx: &EngineContext<'_>, name: &str) -> Result<Value> {
        let (value, owner) = self.read_row(ctx, name).await?;
        self.ensure_runtime_tables().await?;
        self.authorize(
            ctx,
            name,
            authz_owner(ctx, owner.as_deref()),
            Permission::Read,
        )
        .await?;
        Ok(value)
    }
}

fn ensure_reveal_allowed(ctx: &EngineContext<'_>) -> Result<()> {
    if ctx.reveal_secrets {
        Ok(())
    } else {
        Err(Error::Secret(SecretError::RevealNotPermitted))
    }
}

/// Reject a field that is not a declared `Secret` the caller may read.
fn ensure_secret_field(ctx: &EngineContext<'_>, field: &str) -> Result<()> {
    let not_secret = || {
        Error::Secret(SecretError::NotSecretField {
            doctype: ctx.doctype.clone(),
            field: field.to_string(),
        })
    };
    let meta = ctx.meta.ok_or_else(not_secret)?;
    let declared = meta
        .fields
        .iter()
        .find(|candidate| candidate.fieldname == field)
        .ok_or_else(not_secret)?;
    if declared.fieldtype != "Secret" {
        return Err(not_secret());
    }
    if !field_read_allowed(ctx, declared) {
        return Err(Error::Authz(ferris_core::AuthzError::PermissionDenied {
            action: Permission::Read.as_str().to_string(),
            resource: format!("{} {field} (permlevel {})", ctx.doctype, declared.permlevel),
        }));
    }
    Ok(())
}

fn decrypt_field(
    ctx: &EngineContext<'_>,
    stored: &Value,
    field: &str,
) -> Result<Option<SecretValue>> {
    match stored.get(field).and_then(Value::as_str) {
        None | Some("") => Ok(None),
        Some(envelope) => {
            let scope = SecretScope {
                doctype: &ctx.doctype,
                field,
            };
            ferris_core::secret::decrypt_secret(scope, envelope).map(Some)
        }
    }
}
