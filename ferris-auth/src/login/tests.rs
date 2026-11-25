//! Tests for the login backend, the session user stamp and the database
//! session store.
//!
//! The suite uses a deterministic TOTP secret (the RFC 6238 ASCII seed) and
//! generates the code through the same `totp-rs` API the backend verifies with,
//! so no clock manipulation is needed.

use std::collections::HashMap;

use axum_login::{AuthUser, AuthnBackend};
use serde_json::json;
use time::{Duration, OffsetDateTime};
use totp_rs::{Builder, Totp};
use tower_sessions::{
    ExpiredDeletion, SessionStore,
    session::{Id, Record},
};

use super::*;
use crate::UserType;
use crate::password::PasswordConfig;
use crate::user_store::UserStore;

const EMAIL: &str = "login@local";
const PASSWORD: &str = "ferrisAdmin123";
const NEW_PASSWORD: &str = "ferrisAdmin456";

/// 20 ASCII bytes: 160 bits, the length RFC 4226 recommends.
const TOTP_SEED: &[u8; 20] = b"12345678901234567890";

fn credentials(password: &str, totp_code: Option<&str>) -> Credentials {
    Credentials {
        email: EMAIL.to_string(),
        password: password.to_string(),
        totp_code: totp_code.map(str::to_string),
    }
}

fn totp_for(seed: &[u8]) -> Totp {
    Builder::new()
        .with_secret(seed.to_vec())
        .with_digits(6)
        .with_step_duration(30)
        .with_skew(1)
        .build()
        .expect("the RFC 6238 seed is a valid TOTP secret")
}

/// A backend, its store, and one active account.
async fn setup() -> (Backend, UserStore) {
    // `mfa_secret` is a `Secret` field, so writing it needs the AEAD keyring.
    crate::user_store::install_test_keyring();
    let db = ferris_db::memory_db().await.unwrap();
    let store = UserStore::new(db.clone());
    store.init().await.unwrap();
    store
        .create(
            EMAIL,
            "Login User",
            Some("login"),
            UserType::SystemUser,
            PASSWORD,
            &["User".to_string()],
        )
        .await
        .unwrap();
    (Backend::new(db), store)
}

/// Turn MFA on for the account and return the checker that mirrors it.
async fn enable_mfa(store: &UserStore, id: &str) -> Totp {
    let totp = totp_for(TOTP_SEED);
    store
        .set_mfa(id, true, Some(&totp.secret().to_base32()), &[])
        .await
        .unwrap();
    totp
}

async fn stored_hash(store: &UserStore) -> String {
    store
        .login_record(EMAIL)
        .await
        .unwrap()
        .expect("the account exists")
        .password_hash
}

#[tokio::test]
async fn password_success_and_failure() {
    let (backend, store) = setup().await;
    let account = store.find_by_login(EMAIL).await.unwrap().unwrap();

    let user = backend
        .authenticate(credentials(PASSWORD, None))
        .await
        .unwrap()
        .expect("the right password logs in");
    assert_eq!(user.id(), account.id);
    assert_eq!(user.email, EMAIL);

    assert!(
        backend
            .authenticate(credentials("wrong-password", None))
            .await
            .unwrap()
            .is_none(),
        "a wrong password must not authenticate"
    );
}

#[tokio::test]
async fn unknown_user_returns_none() {
    let (backend, _store) = setup().await;
    let creds = Credentials {
        email: "nobody@local".to_string(),
        password: PASSWORD.to_string(),
        totp_code: None,
    };
    assert!(backend.authenticate(creds).await.unwrap().is_none());
}

#[tokio::test]
async fn mfa_required_but_missing_code_is_refused() {
    let (backend, store) = setup().await;
    let account = store.find_by_login(EMAIL).await.unwrap().unwrap();
    enable_mfa(&store, &account.id).await;

    assert!(
        backend
            .authenticate(credentials(PASSWORD, None))
            .await
            .unwrap()
            .is_none(),
        "a password alone must not pass an MFA-enabled account"
    );
    assert!(
        backend
            .authenticate(credentials(PASSWORD, Some("")))
            .await
            .unwrap()
            .is_none(),
        "an empty code is not a code"
    );
}

#[tokio::test]
async fn a_valid_totp_code_logs_in() {
    let (backend, store) = setup().await;
    let account = store.find_by_login(EMAIL).await.unwrap().unwrap();
    let totp = enable_mfa(&store, &account.id).await;
    let code = totp.generate_current().to_string();

    let user = backend
        .authenticate(credentials(PASSWORD, Some(&code)))
        .await
        .unwrap()
        .expect("password + current TOTP code logs in");
    assert_eq!(user.id(), account.id);
}

#[tokio::test]
async fn a_wrong_totp_code_is_refused() {
    let (backend, store) = setup().await;
    let account = store.find_by_login(EMAIL).await.unwrap().unwrap();
    enable_mfa(&store, &account.id).await;

    // A code from a different seed at the Unix epoch cannot be inside the
    // verification window, so this is deterministically wrong — unlike
    // "000000", which has a one-in-a-million chance of being valid.
    let bogus = totp_for(b"abcdefghijklmnopqrst").generate(0).to_string();

    assert!(
        backend
            .authenticate(credentials(PASSWORD, Some(&bogus)))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_replayed_totp_step_is_refused() {
    let (backend, store) = setup().await;
    let account = store.find_by_login(EMAIL).await.unwrap().unwrap();
    let totp = enable_mfa(&store, &account.id).await;
    let code = totp.generate_current().to_string();

    assert!(
        backend
            .authenticate(credentials(PASSWORD, Some(&code)))
            .await
            .unwrap()
            .is_some(),
        "the first use of the step succeeds"
    );
    assert!(
        backend
            .authenticate(credentials(PASSWORD, Some(&code)))
            .await
            .unwrap()
            .is_none(),
        "the same code must not be accepted twice"
    );
}

#[tokio::test]
async fn an_old_password_stops_working_after_a_change() {
    let (backend, store) = setup().await;
    let account = store.find_by_login(EMAIL).await.unwrap().unwrap();

    store.set_password(&account.id, NEW_PASSWORD).await.unwrap();

    assert!(
        backend
            .authenticate(credentials(PASSWORD, None))
            .await
            .unwrap()
            .is_none(),
        "the previous password is invalidated"
    );
    assert!(
        backend
            .authenticate(credentials(NEW_PASSWORD, None))
            .await
            .unwrap()
            .is_some(),
        "the new password authenticates"
    );
}

#[tokio::test]
async fn session_auth_hash_changes_when_the_password_changes() {
    let (backend, store) = setup().await;
    let before = backend
        .authenticate(credentials(PASSWORD, None))
        .await
        .unwrap()
        .expect("initial login");
    let before_hash = before.session_auth_hash().to_vec();

    let account = store.find_by_login(EMAIL).await.unwrap().unwrap();
    store.set_password(&account.id, NEW_PASSWORD).await.unwrap();

    let after = backend
        .get_user(&account.id)
        .await
        .unwrap()
        .expect("the account still exists");
    assert_ne!(
        before_hash,
        after.session_auth_hash().to_vec(),
        "a password change must invalidate the session stamp"
    );
}

#[tokio::test]
async fn session_auth_hash_changes_when_mfa_is_enabled() {
    let (backend, store) = setup().await;
    let before = backend
        .authenticate(credentials(PASSWORD, None))
        .await
        .unwrap()
        .expect("login without MFA");
    let before_hash = before.session_auth_hash().to_vec();

    let account = store.find_by_login(EMAIL).await.unwrap().unwrap();
    enable_mfa(&store, &account.id).await;

    let after = backend
        .get_user(&account.id)
        .await
        .unwrap()
        .expect("the account still exists");
    assert_ne!(
        before_hash,
        after.session_auth_hash().to_vec(),
        "enabling MFA must invalidate the session stamp"
    );
}

#[tokio::test]
async fn the_session_stamp_is_stable_across_reads() {
    let (backend, store) = setup().await;
    let first = backend
        .authenticate(credentials(PASSWORD, None))
        .await
        .unwrap()
        .expect("login");
    let account = store.find_by_login(EMAIL).await.unwrap().unwrap();
    let reloaded = backend.get_user(&account.id).await.unwrap().unwrap();
    assert_eq!(
        first.session_auth_hash(),
        reloaded.session_auth_hash(),
        "the backend must recompute the same stamp on the next request"
    );
}

#[tokio::test]
async fn a_stale_password_hash_is_rehashed_on_login() {
    let db = ferris_db::memory_db().await.unwrap();
    let store = UserStore::new(db.clone());
    store.init().await.unwrap();
    store
        .create(
            EMAIL,
            "Login User",
            None,
            UserType::SystemUser,
            PASSWORD,
            &[],
        )
        .await
        .unwrap();
    let before = stored_hash(&store).await;

    // A cost different from the installed hasher makes the stored hash stale.
    // The rehash itself writes through that installed hasher (default cost),
    // which is enough to observe: Argon2 salts every derivation, so the PHC
    // string changes even when the parameters do not.
    let stricter = PasswordConfig {
        argon2_memory_kib: PasswordConfig::default().argon2_memory_kib * 2,
        ..PasswordConfig::default()
    };
    let backend = Backend::new(db).with_password_config(stricter);
    let user = backend
        .authenticate(credentials(PASSWORD, None))
        .await
        .unwrap()
        .expect("the login succeeds");
    assert_eq!(
        user.id(),
        store.find_by_login(EMAIL).await.unwrap().unwrap().id
    );

    let after = stored_hash(&store).await;
    assert_ne!(before, after, "the stale hash must be re-derived on login");

    // The session issued by that login already carries the new stamp, so the
    // next request does not invalidate it.
    let reloaded = backend.get_user(&user.id()).await.unwrap().unwrap();
    assert_eq!(user.session_auth_hash(), reloaded.session_auth_hash());
}

/// The password and the TOTP code are credentials: a `{:?}` of the submitted
/// form (a handler logging its request, a failing assertion) must not publish
/// them.
#[test]
fn credentials_debug_redacts_both_secrets() {
    let creds = Credentials {
        email: EMAIL.to_string(),
        password: "hunter2".to_string(),
        totp_code: Some("123456".to_string()),
    };
    let debug = format!("{creds:?}");

    assert!(!debug.contains("hunter2"), "{debug}");
    assert!(!debug.contains("123456"), "{debug}");
    assert!(debug.contains(EMAIL), "{debug}");
    assert!(debug.contains("<redacted>"), "{debug}");
}

/// The session validity stamp is the value `axum-login` compares on every
/// request; it must not travel through a `Debug` print of the session user.
#[tokio::test]
async fn session_user_debug_redacts_the_auth_hash() {
    let (backend, _store) = setup().await;
    let user = backend
        .authenticate(credentials(PASSWORD, None))
        .await
        .unwrap()
        .expect("the login succeeds");
    let debug = format!("{user:?}");

    assert!(
        !debug.contains(&format!("{:?}", user.session_auth_hash())),
        "{debug}"
    );
    assert!(debug.contains("<redacted>"), "{debug}");
    assert!(debug.contains(EMAIL), "{debug}");
}

/// A session record without the hard deadline is **not** live. The sliding idle
/// window can be refreshed forever, so accepting a record with no absolute
/// expiry would leave a legacy or forged session valid indefinitely.
#[tokio::test]
async fn a_session_without_the_absolute_deadline_is_not_live() {
    use std::sync::Arc;

    let store = Arc::new(DbSessionStore::new(ferris_db::memory_db().await.unwrap()));
    let session = tower_sessions::Session::new(None, store, None);

    assert!(
        !session_is_live(&session).await,
        "a session with no deadline is refused"
    );

    let deadline = crate::login::keys::ABSOLUTE_EXPIRY_KEY;
    session
        .insert(deadline, chrono::Utc::now().timestamp() + 60)
        .await
        .unwrap();
    assert!(session_is_live(&session).await, "inside the deadline");

    session
        .insert(deadline, chrono::Utc::now().timestamp() - 1)
        .await
        .unwrap();
    assert!(
        !session_is_live(&session).await,
        "a deadline in the past is dead"
    );
}

// --- session store ---------------------------------------------------------

fn record(data: &[(&str, serde_json::Value)]) -> Record {
    let now = OffsetDateTime::now_utc();
    // Whole seconds: the column stores a Unix timestamp, so sub-second
    // precision would not survive the round trip.
    let expiry_date =
        OffsetDateTime::from_unix_timestamp(now.unix_timestamp()).unwrap() + Duration::minutes(5);
    Record {
        id: Id::default(),
        data: data
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect::<HashMap<_, _>>(),
        expiry_date,
    }
}

#[tokio::test]
async fn session_store_save_load_delete_round_trip() {
    let db = ferris_db::memory_db().await.unwrap();
    let store = DbSessionStore::new(db.clone());

    let mut created = record(&[("theme", json!("dark"))]);
    store.create(&mut created).await.unwrap();

    let loaded = store
        .load(&created.id)
        .await
        .unwrap()
        .expect("the record is stored");
    assert_eq!(loaded, created);

    let mut updated = loaded;
    updated
        .data
        .insert("lang".to_string(), json!({ "tag": "es" }));
    store.save(&updated).await.unwrap();
    assert_eq!(store.load(&created.id).await.unwrap().unwrap(), updated);

    // The data column is JSON text, not a blob of Rust debug output.
    let raw = db
        .fetch_one(&ferris_db::Stmt::new(
            format!("SELECT data FROM {SESSION_TABLE} WHERE id = ?"),
            vec![ferris_core::Value::from(created.id.to_string())],
        ))
        .await
        .unwrap();
    let text = raw.str("data").unwrap();
    assert!(text.starts_with('{'), "expected a JSON object, got {text}");
    assert!(text.contains("\"lang\""));

    store.delete(&created.id).await.unwrap();
    assert!(store.load(&created.id).await.unwrap().is_none());
}

#[tokio::test]
async fn an_expired_record_is_not_returned() {
    let db = ferris_db::memory_db().await.unwrap();
    let store = DbSessionStore::new(db);

    let mut expired = record(&[("k", json!(1))]);
    expired.expiry_date = OffsetDateTime::now_utc() - Duration::seconds(1);
    store.create(&mut expired).await.unwrap();

    assert!(
        store.load(&expired.id).await.unwrap().is_none(),
        "a row past its expiry must not rehydrate a session"
    );
}

#[tokio::test]
async fn delete_expired_sweeps_only_dead_rows() {
    let db = ferris_db::memory_db().await.unwrap();
    let store = DbSessionStore::new(db);

    let mut live = record(&[("k", json!("live"))]);
    let mut dead = record(&[("k", json!("dead"))]);
    dead.expiry_date = OffsetDateTime::now_utc() - Duration::minutes(1);
    store.create(&mut live).await.unwrap();
    store.create(&mut dead).await.unwrap();

    store.delete_expired().await.unwrap();

    assert!(store.load(&live.id).await.unwrap().is_some());
    assert!(
        store.load(&dead.id).await.unwrap().is_none(),
        "the expired row is gone"
    );
}

#[tokio::test]
async fn the_table_is_created_lazily_and_idempotently() {
    let db = ferris_db::memory_db().await.unwrap();
    // Two stores over the same database: both ensure the table, neither fails.
    let first = DbSessionStore::new(db.clone());
    let second = DbSessionStore::new(db);
    first.ensure_table().await.unwrap();
    second.ensure_table().await.unwrap();

    let mut created = record(&[("k", json!(true))]);
    second.create(&mut created).await.unwrap();
    assert!(first.load(&created.id).await.unwrap().is_some());
}

#[tokio::test]
async fn a_reused_id_is_replaced_on_create() {
    let db = ferris_db::memory_db().await.unwrap();
    let store = DbSessionStore::new(db);

    let mut first = record(&[("owner", json!("first"))]);
    store.create(&mut first).await.unwrap();

    let mut collision = first.clone();
    collision.data = HashMap::from([("owner".to_string(), json!("second"))]);
    store.create(&mut collision).await.unwrap();

    assert_ne!(
        collision.id, first.id,
        "the store must not overwrite a live session"
    );
    assert_eq!(
        store.load(&first.id).await.unwrap().unwrap().data,
        first.data
    );
    assert_eq!(
        store.load(&collision.id).await.unwrap().unwrap().data,
        collision.data
    );
}
