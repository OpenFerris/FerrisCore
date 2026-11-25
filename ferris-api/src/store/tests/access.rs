use ferris_core::{Error, Value};
use ferris_document::{DocStatus, DocumentEngine, EngineContext};

use crate::test_support::register_probe;
use crate::types::ListParams;

use super::super::meta::context;
use super::super::{DocStore, document_exists, effective_meta, ensure_shareable};
use super::harness::{admin, pool};

/// H5 regression: a share persisted by the API layer is honoured by the
/// engine on the next request. The old wiring wrote `tab_share` rows that no
/// production engine ever read, so the invited user still got a 403; this
/// test creates the document, grants the share the `POST …/shares` handler
/// writes, and reads as the guest.
#[tokio::test]
async fn a_persisted_share_grants_get_and_list_to_the_invited_user() {
    use ferris_document::sharing::{ShareManager, SharePermission};

    register_probe();
    let pool = pool().await;
    DocStore::init(&pool).unwrap();
    ferris_document::with_user(Some(admin()), async {
        DocStore::create(
            &pool,
            "Probe",
            ferris_core::value!({ "name": "SHARED-1", "title": "secret" }),
        )
        .await
        .unwrap();
    })
    .await;

    let shares = ShareManager::with_db(pool.clone());
    shares.init().await.unwrap();
    shares
        .share(
            "Probe",
            "SHARED-1",
            "alice@local",
            vec![SharePermission::Read],
            "admin@local",
        )
        .await
        .unwrap();

    let mut alice = ferris_auth::User::system_user("alice@local".to_string(), "Alice".to_string());
    alice.id = "alice@local".to_string();
    ferris_document::with_user(Some(alice.clone()), async {
        let doc = DocStore::get(&pool, "Probe", "SHARED-1")
            .await
            .expect("the stored share grants Read");
        assert_eq!(doc.get("title").and_then(Value::as_str), Some("secret"));

        let listed = DocStore::list(&pool, "Probe", &ListParams::default())
            .await
            .unwrap();
        assert_eq!(listed.total, 1, "the shared row is in the guest's list");

        // A read share is not a share right: the guest cannot re-share it.
        let err = ensure_shareable(&pool, "Probe", "SHARED-1")
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Authz(_)), "got {err:?}");
    })
    .await;

    let mut bob = ferris_auth::User::system_user("bob@local".to_string(), "Bob".to_string());
    bob.id = "bob@local".to_string();
    ferris_document::with_user(Some(bob), async {
        let err = DocStore::get(&pool, "Probe", "SHARED-1").await.unwrap_err();
        assert!(
            matches!(err, Error::Authz(_)),
            "a share must not widen access for an unrelated user: {err:?}"
        );
    })
    .await;

    ferris_document::with_user(Some(admin()), async {
        ensure_shareable(&pool, "Probe", "SHARED-1")
            .await
            .expect("the owner with the share right may manage shares");
    })
    .await;
}

/// `document_exists` must report a denied read as an error, not
/// as "no row". The upsert sites used `engine.get(..).await.is_ok()` and so
/// fell into their `create` branch whenever the read was refused.
#[tokio::test]
async fn document_exists_distinguishes_denied_read_from_missing_row() {
    let pool = pool().await;
    DocStore::init(&pool).unwrap();
    let engine = DocumentEngine::new(pool.clone());

    let mut bob = ferris_auth::User::system_user("bob@local".to_string(), "Bob".to_string());
    bob.id = "bob@local".to_string();
    let owner_ctx = {
        let mut ctx = EngineContext::new("Probe", "PRO-.#####");
        ctx.allow_explicit_name = true;
        ctx.user = Some(admin());
        ctx
    };
    engine
        .create_owned_by(
            &owner_ctx,
            &bob.id,
            ferris_core::value!({ "name": "SECRET-1" }),
        )
        .await
        .unwrap();

    // A genuinely missing row is `Ok(false)`.
    ferris_document::with_user(Some(admin()), async {
        let ctx = context("Probe", "PRO-.#####", None);
        assert!(!document_exists(&engine, &ctx, "NOPE-1").await.unwrap());
    })
    .await;

    // A row that exists but is not readable is an error, never `Ok(false)`.
    let mut mallory = ferris_auth::User::system_user("mallory@local".to_string(), "M".to_string());
    mallory.id = "mallory@local".to_string();
    ferris_document::with_user(Some(mallory), async {
        let ctx = context("Probe", "PRO-.#####", None);
        let err = document_exists(&engine, &ctx, "SECRET-1")
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
            ),
            "a denied read must not read as a missing row: {err:?}"
        );
    })
    .await;
}

/// §C2 — the REST read routes used to serve every document to every
/// authenticated caller. These are the HTTP-level regression tests.
#[tokio::test]
async fn rest_reads_do_not_leak_documents_of_other_owners() {
    register_probe();
    let pool = pool().await;
    DocStore::init(&pool).unwrap();

    let mut victim = ferris_auth::User::system_user("bob@local".to_string(), "Bob".to_string());
    victim.id = "bob@local".to_string();
    let mut mallory = ferris_auth::User::system_user("mallory@local".to_string(), "M".to_string());
    mallory.id = "mallory@local".to_string();

    // Bob owns SECRET-1; Mallory owns PUBLIC-1.
    let admin_ctx = {
        let mut ctx = EngineContext::new("Probe", "PRO-.#####");
        ctx.allow_explicit_name = true;
        ctx.user = Some(admin());
        ctx
    };
    ferris_document::with_user(Some(admin()), async {
        for (owner, name, title) in [
            ("bob@local", "SECRET-1", "bob's salary"),
            ("mallory@local", "PUBLIC-1", "mallory's"),
        ] {
            DocumentEngine::new(pool.clone())
                .create_owned_by(
                    &admin_ctx,
                    owner,
                    ferris_core::value!({ "name": name, "title": title }),
                )
                .await
                .unwrap();
        }
    })
    .await;

    // Mallory cannot read Bob's document by name.
    ferris_document::with_user(Some(mallory.clone()), async {
        let err = DocStore::get(&pool, "Probe", "SECRET-1").await.unwrap_err();
        assert!(
            matches!(
                err,
                Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
            ),
            "IDOR: mallory read bob's document: {err:?}"
        );

        // ...and the list route withholds it, with an honest `total`.
        let listed = DocStore::list(&pool, "Probe", &ListParams::default())
            .await
            .unwrap();
        assert_eq!(listed.total, 1, "total leaked the hidden document");
        assert_eq!(listed.data.len(), 1);
        assert_eq!(
            listed.data[0].get("name").and_then(|v| v.as_str()),
            Some("PUBLIC-1")
        );
    })
    .await;

    // Bob reads his own.
    ferris_document::with_user(Some(victim), async {
        let doc = DocStore::get(&pool, "Probe", "SECRET-1").await.unwrap();
        assert_eq!(
            doc.get("title").and_then(|v| v.as_str()),
            Some("bob's salary")
        );
    })
    .await;
}

/// §C2 follow-up — `amend` used to read its source with no check at all, so
/// it both disclosed and copied any document in the installation. With
/// `Read` enforced it still needed `Amend`, or any reader could duplicate a
/// submitted document.
#[tokio::test]
async fn rest_amend_requires_amend_permission() {
    register_probe();
    let pool = pool().await;
    DocStore::init(&pool).unwrap();

    let mut victim = ferris_auth::User::system_user("bob@local".to_string(), "Bob".to_string());
    victim.id = "bob@local".to_string();
    let mut mallory = ferris_auth::User::system_user("mallory@local".to_string(), "M".to_string());
    mallory.id = "mallory@local".to_string();

    let admin_ctx = {
        let mut ctx = EngineContext::new("Probe", "PRO-.#####");
        ctx.allow_explicit_name = true;
        ctx.user = Some(admin());
        ctx
    };
    ferris_document::with_user(Some(admin()), async {
        // Owned by Bob; a `System Manager` submits it. Bob can amend his own
        // submitted document via the owner shortcut, Mallory cannot.
        DocumentEngine::new(pool.clone())
            .create_owned_by(
                &admin_ctx,
                "bob@local",
                ferris_core::value!({ "name": "S-1", "title": "submitted" }),
            )
            .await
            .unwrap();
        DocStore::set_docstatus(&pool, "Probe", "S-1", DocStatus::Submitted)
            .await
            .unwrap();
        // Frappe only amends a cancelled document; leave the row in that state
        // for the Amend gate assertions below (DOC-02).
        DocStore::set_docstatus(&pool, "Probe", "S-1", DocStatus::Cancelled)
            .await
            .unwrap();
    })
    .await;

    ferris_document::with_user(Some(mallory), async {
        let err = DocStore::amend(&pool, "Probe", "S-1").await.unwrap_err();
        // Assert the *action*, not merely that it failed: with the source
        // read unchecked this same call still fails, but at the `create`
        // gate, which is a different bug wearing the same error type. Only
        // the action string tells the two apart.
        match &err {
            Error::Authz(ferris_core::AuthzError::PermissionDenied { action, .. }) => {
                assert_eq!(action, "amend", "refused at the wrong gate: {err:?}")
            }
            _ => panic!("anyone could clone a cancelled document: {err:?}"),
        }
    })
    .await;

    // The owner gets past the `Amend` gate, but the copy it then tries to
    // create is refused: with no policies installed (§C3) the only principal
    // who may `Create` anything is a `System Manager`, because a brand-new
    // document has no owner for the owner shortcut to match. So Bob's
    // happy path cannot be asserted until §C3 lands — the important half,
    // that the *read* is refused before the copy is even attempted, is the
    // assertion above.
    ferris_document::with_user(Some(victim.clone()), async {
        let err = DocStore::amend(&pool, "Probe", "S-1").await.unwrap_err();
        match &err {
            Error::Authz(ferris_core::AuthzError::PermissionDenied { action, .. }) => {
                assert_eq!(action, "create", "the Amend gate must pass first: {err:?}")
            }
            _ => panic!("the owner holds Amend via the owner shortcut: {err:?}"),
        }
    })
    .await;
    let _ = victim;

    // A `System Manager` amends end to end: gate passed, copy created.
    ferris_document::with_user(Some(admin()), async {
        let amended = DocStore::amend(&pool, "Probe", "S-1").await.unwrap();
        assert_eq!(
            amended.get("name").and_then(|v| v.as_str()),
            Some("S-1-1"),
            "the amended copy must be named after its source plus a counter"
        );
        assert_eq!(
            amended.get("docstatus").and_then(|v| v.as_str()),
            Some("draft")
        );
    })
    .await;
}

/// The neighbours belong to a document, so the same `Read` gate as
/// `GET /api/resource/{doctype}/{name}` applies: a caller who cannot read
/// the document is refused rather than told what sits next to it.
#[tokio::test]
async fn neighbors_require_read_on_the_document() {
    let pool = pool().await;
    ferris_document::with_user(Some(admin()), async {
        let meta = effective_meta(&pool, "Probe").await.unwrap();
        let ctx = {
            let mut ctx = EngineContext::new("Probe", "Q-.#####");
            ctx.allow_explicit_name = true;
            ctx.meta = meta.as_ref();
            ctx.user = Some(admin());
            ctx
        };
        DocumentEngine::new(pool.clone())
            .create_owned_by(&ctx, "bob@local", ferris_core::value!({ "name": "Q-1" }))
            .await
            .unwrap();

        let mut mallory =
            ferris_auth::User::system_user("mallory@local".to_string(), "M".to_string());
        mallory.id = "mallory@local".to_string();
        ferris_document::with_user(Some(mallory), async {
            let err = DocStore::neighbors(&pool, "Probe", "Q-1")
                .await
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::Authz(ferris_core::AuthzError::PermissionDenied { .. })
                ),
                "the neighbours of an unreadable document must be refused: {err:?}"
            );
        })
        .await;
    })
    .await;
}
