//! Fixtures shared by the API crate's unit tests.
//!
//! Columns are the only document storage (ADR 0001), so a test that writes a
//! declared field has to register the `DocType` first: an unregistered name
//! only has the system columns. `Probe` is the fixture the store, activity and
//! data tests drive through `DocStore` and the engine; this is its single
//! definition, so the suites cannot disagree about its fields.

use ferris_schema::{DoctypeMeta, FieldMeta};

/// Register the `Probe` fixture used by the API tests.
///
/// Idempotent: the registry replaces by name, and every caller installs the
/// same definition, so parallel tests cannot race into a different shape.
pub(crate) fn register_probe() {
    ferris_schema::register_doctype(DoctypeMeta {
        name: "Probe".to_string(),
        module: "test".to_string(),
        fields: vec![
            FieldMeta::data("title"),
            FieldMeta {
                fieldname: "qty".to_string(),
                fieldtype: "Int".to_string(),
                ..Default::default()
            },
            FieldMeta::data("status"),
        ],
        ..Default::default()
    });
    // The registration is the base metadata `effective_meta` merges over. A
    // pool that already cached the old shape must reload it.
    ferris_schema::CustomizationStore::invalidate_meta("Probe");
}

/// Install the fixed AEAD keyring the MFA tests seal `User.mfa_secret` with.
///
/// `mfa_secret` is a `Secret` field, so writing one needs a keyring; without it
/// every write fails closed with `SecretError::KeyMissing`. `install_keyring`
/// treats a re-install of the same material as a no-op, so every test module
/// can call this. The same fixed key is used in `ferris-auth`'s own tests; it is
/// test material and must never reach a deployment.
pub(crate) fn install_test_keyring() {
    const TEST_SECRET_KEY: &str =
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    let key = ferris_core::secret::SecretKey::parse(TEST_SECRET_KEY)
        .expect("the test key is 32 bytes of hex");
    let _ = ferris_core::secret::install_keyring(ferris_core::secret::SecretKeyring::new(
        key,
        Vec::new(),
    ));
}
