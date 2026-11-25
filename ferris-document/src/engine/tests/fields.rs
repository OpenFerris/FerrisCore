use super::*;

const MASK: &str = ferris_core::password::PASSWORD_MASK;

/// A DocType with one computed field
/// (<https://docs.frappe.io/framework/user/en/basics/virtual_docfield>).
fn virtual_meta() -> DoctypeMeta {
    DoctypeMeta {
        name: "PersonV".to_string(),
        fields: vec![
            field("first", "Data", 0),
            field("last", "Data", 0),
            ferris_schema::FieldMeta {
                fieldname: "full_name".to_string(),
                fieldtype: "Data".to_string(),
                r#virtual: true,
                ..Default::default()
            },
            ferris_schema::FieldMeta {
                fieldname: "virtual_lines".to_string(),
                fieldtype: "Table".to_string(),
                r#virtual: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// A virtual child table is the same provider contract: an array of rows.
fn virtual_lines(_document: &Value) -> ferris_core::Result<Value> {
    Ok(Value::Array(vec![ferris_core::value!({
        "label": "computed",
        "amount": 2
    })]))
}

fn full_name(document: &Value) -> ferris_core::Result<Value> {
    let first = document
        .get("first")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let last = document
        .get("last")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Ok(Value::from(format!("{first} {last}").trim().to_string()))
}

/// A Virtual DocField is computed on read and refused on write: persisting it
/// would be silently overwritten by the next read, which is exactly the
/// "silent success" the engine must not allow.
#[tokio::test]
async fn a_virtual_field_is_computed_on_read_and_read_only_on_write() {
    crate::register_virtual_field("PersonV", "full_name", full_name);
    crate::register_virtual_field("PersonV", "virtual_lines", virtual_lines);
    let engine = engine().await;
    let meta = virtual_meta();
    let ctx = EngineContext::new("PersonV", "")
        .with_meta(&meta)
        .allow_anonymous(true);

    let error = engine
        .create(
            &ctx,
            ferris_core::value!({ "first": "Ada", "last": "Lovelace", "full_name": "nope" }),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Document(DocumentError::VirtualFieldReadOnly { ref field, .. }) if field == "full_name"
    ));

    let created = engine
        .create(
            &ctx,
            ferris_core::value!({ "first": "Ada", "last": "Lovelace" }),
        )
        .await
        .unwrap();
    let name = created
        .get("name")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let read = engine.get(&ctx, &name).await.unwrap();
    assert_eq!(
        read.get("full_name").and_then(Value::as_str),
        Some("Ada Lovelace")
    );
    // The write response is a document read like any other, so it carries the
    // computed value too (Frappe's `get_valid_dict` does the same); what must
    // never happen is *persisting* it.
    assert_eq!(
        created.get("full_name").and_then(Value::as_str),
        Some("Ada Lovelace")
    );
    // A virtual child table is an array computed at read time, with no stored
    // child rows.
    let lines = read
        .get("virtual_lines")
        .and_then(Value::as_array)
        .expect("the virtual child table is computed on read");
    assert_eq!(lines.len(), 1);
    assert_eq!(
        lines[0].get("label").and_then(Value::as_str),
        Some("computed")
    );
}

#[tokio::test]
async fn field_permissions_hide_admin_fields_and_mask_passwords() {
    install_test_hasher();
    let engine = engine().await;
    let meta = vault_meta();
    let ctx = EngineContext::new("Vault", "VAULT-.#####")
        .with_meta(&meta)
        .allow_anonymous(true);

    let created = engine
        .create(
            &ctx,
            ferris_core::value!({
                "title": "t",
                "admin_note": "secret",
                "password": "hunter2"
            }),
        )
        .await
        .unwrap();

    // The caller could not write the level-1 field, so it was dropped...
    assert!(
        created.get("admin_note").is_none(),
        "a level-1 field must not be writable by a caller with no field rule"
    );
    // ...and the password never leaves the engine in clear.
    assert_eq!(created.get("password").and_then(|v| v.as_str()), Some(MASK));

    let name = created.get("name").unwrap().as_str().unwrap().to_string();
    // The stored blob holds only the hash, and never received the admin note.
    let (stored, _owner) = engine.read_row(&ctx, &name).await.unwrap();
    assert!(stored.get("admin_note").is_none());
    assert_eq!(
        stored.get("password").and_then(|v| v.as_str()),
        Some("hashed:hunter2")
    );

    // A plain read redacts the same way.
    let fetched = engine.get(&ctx, &name).await.unwrap();
    assert!(fetched.get("admin_note").is_none());
    assert_eq!(fetched.get("password").and_then(|v| v.as_str()), Some(MASK));
}

#[tokio::test]
async fn system_manager_sees_admin_fields_but_never_the_password_hash() {
    install_test_hasher();
    let engine = engine().await;
    let meta = vault_meta();
    let admin = sysadmin();
    let ctx = EngineContext::new("Vault", "VAULT-.#####")
        .with_meta(&meta)
        .with_user(&admin);

    let created = engine
        .create(
            &ctx,
            ferris_core::value!({
                "title": "t",
                "admin_note": "n",
                "password": "hunter2"
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        created.get("admin_note").and_then(|v| v.as_str()),
        Some("n")
    );
    // A role must not turn into a hash oracle: the API read path masks the
    // field for System Manager too.
    assert_eq!(created.get("password").and_then(|v| v.as_str()), Some(MASK));

    let name = created.get("name").unwrap().as_str().unwrap().to_string();
    let (stored, _) = engine.read_row(&ctx, &name).await.unwrap();
    assert_eq!(
        stored.get("password").and_then(|v| v.as_str()),
        Some("hashed:hunter2")
    );

    let fetched = engine.get(&ctx, &name).await.unwrap();
    assert_eq!(
        fetched.get("admin_note").and_then(|v| v.as_str()),
        Some("n")
    );
    assert_eq!(fetched.get("password").and_then(|v| v.as_str()), Some(MASK));

    // Only the authentication store, which opts in explicitly, may read the
    // stored hash through the engine.
    let trusted = EngineContext::new("Vault", "VAULT-.#####")
        .with_meta(&meta)
        .with_user(&admin)
        .reveal_password_hash();
    let raw = engine.get(&trusted, &name).await.unwrap();
    assert_eq!(
        raw.get("password").and_then(|v| v.as_str()),
        Some("hashed:hunter2")
    );
}

#[tokio::test]
async fn update_cannot_move_a_field_beyond_the_callers_level() {
    install_test_hasher();
    let engine = engine().await;
    let meta = vault_meta();
    let admin = sysadmin();
    let admin_ctx = EngineContext::new("Vault", "VAULT-.#####")
        .with_meta(&meta)
        .with_user(&admin);

    let created = engine
        .create(
            &admin_ctx,
            ferris_core::value!({ "title": "t", "admin_note": "n1" }),
        )
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();

    // An anonymous writer may edit the level-0 field but not the level-1 one.
    let open = EngineContext::new("Vault", "VAULT-.#####")
        .with_meta(&meta)
        .allow_anonymous(true);
    let updated = engine
        .update(
            &open,
            &name,
            ferris_core::value!({ "title": "t2", "admin_note": "n2" }),
        )
        .await
        .unwrap();
    assert_eq!(updated.get("title").and_then(|v| v.as_str()), Some("t2"));
    assert!(updated.get("admin_note").is_none());

    // The stored admin note is untouched.
    let (stored, _owner) = engine.read_row(&open, &name).await.unwrap();
    assert_eq!(
        stored.get("admin_note").and_then(|v| v.as_str()),
        Some("n1")
    );
}

/// A `Password` write that echoes the read mask means "unchanged": the stored
/// hash survives rather than being replaced by the mask.
#[tokio::test]
async fn a_password_mask_on_write_keeps_the_stored_hash() {
    install_test_hasher();
    let engine = engine().await;
    let meta = vault_meta();
    let admin = sysadmin();
    let ctx = EngineContext::new("Vault", "VAULT-.#####")
        .with_meta(&meta)
        .with_user(&admin);

    let created = engine
        .create(
            &ctx,
            ferris_core::value!({ "title": "t", "password": "hunter2" }),
        )
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();

    // Echo back the mask the read handed out.
    engine
        .update(&ctx, &name, ferris_core::value!({ "password": MASK }))
        .await
        .unwrap();

    let (stored, _owner) = engine.read_row(&ctx, &name).await.unwrap();
    assert_eq!(
        stored.get("password").and_then(|v| v.as_str()),
        Some("hashed:hunter2"),
        "a masked password must not overwrite the stored hash"
    );
}

// ---------------------------------------------------------------------------
// Recoverable `Secret` fields (H3)
//
// A `Secret` is the *recoverable* counterpart of `Password`: encrypted at rest
// so a connector can use it, masked on every ordinary read, and decrypted only
// by `read_secret` on a context that opted in. The fail-closed branch with no
// keyring installed is covered deterministically in
// `ferris_core::secret::tests` (`encrypt_with(None, …)`); the keyring here is
// process-wide and cannot be reset, so these tests assert the observable
// contract instead.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_secret_round_trips_only_through_the_revealed_reader() {
    install_test_hasher();
    install_test_keyring();
    let engine = engine().await;
    let meta = secret_meta();
    let ctx = EngineContext::new("VaultS", "")
        .with_meta(&meta)
        .allow_anonymous(true);

    let created = engine
        .create(
            &ctx,
            ferris_core::value!({
                "title": "t",
                "password": "hunter2",
                "client_secret": "s3cret"
            }),
        )
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();

    // The write response is a read: the secret is masked, never echoed.
    assert_eq!(
        created.get("client_secret").and_then(Value::as_str),
        Some(MASK)
    );

    // Stored: an envelope, and the plaintext is not in it.
    let (stored, _owner) = engine.read_row(&ctx, &name).await.unwrap();
    let envelope = stored
        .get("client_secret")
        .and_then(Value::as_str)
        .expect("stored envelope");
    assert!(envelope.starts_with("ferris:v1:"), "{envelope}");
    assert!(!envelope.contains("s3cret"), "{envelope}");

    // An ordinary read masks it, including for a System Manager.
    let admin = sysadmin();
    let admin_ctx = EngineContext::new("VaultS", "")
        .with_meta(&meta)
        .with_user(&admin);
    let fetched = engine.get(&admin_ctx, &name).await.unwrap();
    assert_eq!(
        fetched.get("client_secret").and_then(Value::as_str),
        Some(MASK)
    );

    // Without the explicit opt-in the plaintext is not reachable at all.
    let error = engine
        .read_secret(&admin_ctx, &name, "client_secret")
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::Secret(SecretError::RevealNotPermitted)),
        "{error:?}"
    );

    // With it, the caller gets a SecretValue — and the *document* still masks.
    let trusted = EngineContext::new("VaultS", "")
        .with_meta(&meta)
        .with_user(&admin)
        .reveal_secrets();
    let secret = engine
        .read_secret(&trusted, &name, "client_secret")
        .await
        .expect("decrypts")
        .expect("the field is set");
    assert_eq!(secret.expose(), "s3cret");
    assert!(!format!("{secret:?}").contains("s3cret"));
    let revealed_document = engine.get(&trusted, &name).await.unwrap();
    assert_eq!(
        revealed_document
            .get("client_secret")
            .and_then(Value::as_str),
        Some(MASK),
        "reveal_secrets unlocks read_secret, never the document"
    );

    // `Password` keeps its own contract: still hashed, still masked.
    assert_eq!(
        stored.get("password").and_then(Value::as_str),
        Some("hashed:hunter2")
    );
}

#[tokio::test]
async fn a_secret_mask_on_write_keeps_the_stored_value() {
    install_test_hasher();
    install_test_keyring();
    let engine = engine().await;
    let meta = secret_meta();
    let ctx = EngineContext::new("VaultS", "")
        .with_meta(&meta)
        .allow_anonymous(true);

    let created = engine
        .create(
            &ctx,
            ferris_core::value!({ "title": "t", "client_secret": "s3cret" }),
        )
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();
    let (before, _) = engine.read_row(&ctx, &name).await.unwrap();
    let envelope_before = before
        .get("client_secret")
        .and_then(Value::as_str)
        .expect("envelope")
        .to_string();

    engine
        .update(
            &ctx,
            &name,
            ferris_core::value!({ "title": "t2", "client_secret": MASK }),
        )
        .await
        .unwrap();
    let (after, _) = engine.read_row(&ctx, &name).await.unwrap();
    assert_eq!(
        after.get("client_secret").and_then(Value::as_str),
        Some(envelope_before.as_str()),
        "a masked secret must not overwrite the stored value"
    );

    engine
        .update(
            &ctx,
            &name,
            ferris_core::value!({ "client_secret": "rotated" }),
        )
        .await
        .unwrap();
    let (rotated, _) = engine.read_row(&ctx, &name).await.unwrap();
    assert_ne!(
        rotated.get("client_secret").and_then(Value::as_str),
        Some(envelope_before.as_str())
    );
    let trusted = EngineContext::new("VaultS", "")
        .with_meta(&meta)
        .allow_anonymous(true)
        .reveal_secrets();
    assert_eq!(
        engine
            .read_secret(&trusted, &name, "client_secret")
            .await
            .expect("decrypts")
            .expect("set")
            .expose(),
        "rotated"
    );
}

#[tokio::test]
async fn a_hashed_legacy_value_is_reported_not_returned() {
    install_test_hasher();
    install_test_keyring();
    let engine = engine().await;

    let meta = secret_meta();
    // The column exists (the DocType declares the field), but the value is a
    // legacy Argon2 hash from a row written before the field was a `Secret`.
    // Insert it around the engine so no hook encrypts it.
    engine
        .ensure_table_with("VaultS", Some(&meta))
        .await
        .unwrap();
    engine
        .db()
        .execute(&Stmt::new(
            "INSERT INTO tabVaultS \
             (name, owner, docstatus, creation, modified, title, client_secret) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::from("VAULTS-LEGACY"),
                Value::Null,
                Value::from(0_i64),
                Value::from(1_i64),
                Value::from(1_i64),
                Value::from("legacy"),
                Value::from("$argon2id$v=19$m=65536,t=3,p=4$c2FsdA$aGFzaA"),
            ],
        ))
        .await
        .unwrap();
    let name = "VAULTS-LEGACY".to_string();

    let reader = EngineContext::new("VaultS", "")
        .with_meta(&meta)
        .allow_anonymous(true);
    // A normal read still masks: no hash leaks, no error either.
    let fetched = engine.get(&reader, &name).await.unwrap();
    assert_eq!(
        fetched.get("client_secret").and_then(Value::as_str),
        Some(MASK)
    );

    // A reveal is where the failure is actionable.
    let trusted = EngineContext::new("VaultS", "")
        .with_meta(&meta)
        .allow_anonymous(true)
        .reveal_secrets();
    let error = engine
        .read_secret(&trusted, &name, "client_secret")
        .await
        .unwrap_err();
    match error {
        Error::Secret(SecretError::NotRecoverable { doctype, field }) => {
            assert_eq!(doctype, "VaultS");
            assert_eq!(field, "client_secret");
        }
        other => panic!("expected NotRecoverable, got {other:?}"),
    }
}

#[tokio::test]
async fn writing_an_envelope_is_rejected() {
    install_test_hasher();
    install_test_keyring();
    let engine = engine().await;
    let meta = secret_meta();
    let ctx = EngineContext::new("VaultS", "")
        .with_meta(&meta)
        .allow_anonymous(true);

    let error = engine
        .create(
            &ctx,
            ferris_core::value!({
                "title": "t",
                "client_secret": "ferris:v1:00000000:AAAA"
            }),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::Validation(ValidationError::Invalid(ref msg)) if msg.contains("envelope")),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_secret_at_a_higher_permlevel_is_not_revealed_to_an_unprivileged_caller() {
    install_test_hasher();
    install_test_keyring();
    let engine = engine().await;
    let meta = secret_meta();
    let admin = sysadmin();
    let admin_ctx = EngineContext::new("VaultS", "")
        .with_meta(&meta)
        .with_user(&admin);
    let created = engine
        .create(
            &admin_ctx,
            ferris_core::value!({ "title": "t", "admin_secret": "top" }),
        )
        .await
        .unwrap();
    let name = created.get("name").unwrap().as_str().unwrap().to_string();

    // A plain user cannot read the document's level-1 field, and the revealed
    // reader refuses it for the same reason — being allowed to decrypt is not a
    // substitute for the permlevel check.
    let alice = EngineContext::new("VaultS", "")
        .with_meta(&meta)
        .with_user(&plain_user())
        .reveal_secrets();
    let error = engine
        .read_secret(&alice, &name, "admin_secret")
        .await
        .unwrap_err();
    assert!(denied(&error), "{error:?}");

    // The admin can.
    let value = engine
        .read_secret(
            &EngineContext::new("VaultS", "")
                .with_meta(&meta)
                .with_user(&admin)
                .reveal_secrets(),
            &name,
            "admin_secret",
        )
        .await
        .expect("admin decrypts")
        .expect("set");
    assert_eq!(value.expose(), "top");
}

/// Data Masking: a `mask` field is projected masked unless the caller's role
/// carries the `mask` right at the field's `permlevel`. The projection is the
/// same one `Password`/`Secret` use, and the stored value is untouched.
#[test]
fn a_masked_field_is_projected_masked_without_the_mask_right() {
    let mut viewer = plain_user();
    viewer.roles = vec![ferris_core::permissions::ALL_USER_ROLE.to_string()];

    let mut meta = DoctypeMeta {
        name: "Masked".to_string(),
        fields: vec![field("title", "Data", 0), masked_field("phone", "Phone", 0)],
        permissions: vec![ferris_core::PermissionRule {
            role: ferris_core::permissions::ALL_USER_ROLE.to_string(),
            read: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let ctx = EngineContext::new("Masked", "MASKED-.#####")
        .with_meta(&meta)
        .with_user(&viewer);
    let doc = ferris_core::value!({
        "name": "MASKED-1",
        "title": "t",
        "phone": "555-0100"
    });

    let redacted = crate::engine::support::redact_read(&ctx, doc.clone());
    assert_eq!(
        redacted.get("phone").and_then(Value::as_str),
        Some("********"),
        "a viewer without the mask right sees the placeholder"
    );
    assert_eq!(
        doc.get("phone").and_then(Value::as_str),
        Some("555-0100"),
        "masking is a read projection, never a mutation of the source"
    );

    // Granting `mask` on the same level reveals the real value.
    meta.permissions[0]
        .custom_permissions
        .push("mask".to_string());
    let granted_ctx = EngineContext::new("Masked", "MASKED-.#####")
        .with_meta(&meta)
        .with_user(&viewer);
    let read = crate::engine::support::redact_read(&granted_ctx, doc);
    assert_eq!(read.get("phone").and_then(Value::as_str), Some("555-0100"));
}

fn masked_field(fieldname: &str, fieldtype: &str, permlevel: u8) -> ferris_schema::FieldMeta {
    ferris_schema::FieldMeta {
        fieldname: fieldname.to_string(),
        fieldtype: fieldtype.to_string(),
        permlevel,
        mask: true,
        ..Default::default()
    }
}
