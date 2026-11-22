//! The engine end to end: a DocType policy deciding a list query.

use ferris_schema::get_doctype;

use super::support::*;
use crate::authz::*;

/// §C2 × §C3, end to end through the real engine.
///
/// §C2 made `list()` drop any row the caller may not read. That is correct
/// and it silently broke the desk: an `Event` with no `user` is shared — the
/// panel says so at `panel.rs:301` — but `owner` is stamped to whoever
/// created it, so the owner rule alone showed a shared event to its creator
/// and to nobody else. Declaring `All` read on `Event` in
/// `core_doctypes::event_meta` is what restores it.
///
/// The grant does **not** separate a private event from a shared one, and the
/// test pins that rather than asserting something kinder: `All` read means any
/// signed-in user may read any `Event` document, and the `user` field is
/// narrowed afterwards by the panel (`panel.rs:328-333`), which is a
/// presentation filter and not an authorization boundary. The policy language
/// cannot express "read this document only when its `user` field is me":
/// `PolicyStatement::with_condition` can only read the evaluation context,
/// and `is_owner` is the only key any caller supplies. Recorded as a residual
/// instead of papered over with a rule that does not exist.
///
/// It runs here rather than in `panel.rs` because it installs policies, and
/// `SERIALISE` is this module's lock; a panel-side test would race the tests
/// below for the one process-wide engine.
#[tokio::test]
async fn a_shared_event_reaches_a_user_who_did_not_create_it() {
    let _scope = engine_scope();
    {
        crate::core_doctypes::register_core_doctypes();
        install_policies(&PolicySource::default());

        let pool = ferris_db::memory_db().await.unwrap();
        let engine = ferris_document::DocumentEngine::new(pool);
        let meta = get_doctype(crate::core_doctypes::EVENT).unwrap();

        // Posted by the service identity, as the desk's own notification path
        // does. `Event` grants `All` read but not create, so a plain user
        // cannot invent an event — the correct reading, since nothing in the
        // workspace writes `Event` today.
        let author = service_user();
        let as_author = ferris_document::EngineContext::new(crate::core_doctypes::EVENT, "")
            .with_meta(&meta)
            .with_user(&author)
            .with_explicit_name();
        let author_id = author.id.clone();
        engine
            .create_owned_by(&as_author, &author_id, shared_event("Standup"))
            .await
            .unwrap();

        // Without the `All` grant this list comes back empty: the row belongs
        // to the service identity and `grace` is not it.
        let grace = user("grace@example.com", &[]);
        let as_grace = ferris_document::EngineContext::new(crate::core_doctypes::EVENT, "")
            .with_meta(&meta)
            .with_user(&grace);
        let (docs, total) = engine
            .list(&as_grace, &ferris_document::ListQuery::default())
            .await
            .unwrap();
        assert_eq!(
            total, 1,
            "a shared event must reach a user who did not create it"
        );
        assert_eq!(
            docs[0].get("title").and_then(|v| v.as_str()),
            Some("Standup")
        );
    }
}

/// The inverse, and the assertion that keeps C3 from being read as "the engine
/// now allows everything": with the same `Event` grant installed, a user with
/// no roles at all still cannot read a DocType that declares nothing.
#[tokio::test]
async fn an_undeclared_doctype_stays_closed_to_a_role_less_user() {
    let _scope = engine_scope();
    {
        crate::core_doctypes::register_core_doctypes();
        install_policies(&PolicySource::default());

        let pool = ferris_db::memory_db().await.unwrap();
        let engine = ferris_document::DocumentEngine::new(pool);
        let meta = get_doctype(crate::core_doctypes::NOTIFICATION).unwrap();

        let author = service_user();
        let as_author = ferris_document::EngineContext::new(crate::core_doctypes::NOTIFICATION, "")
            .with_meta(&meta)
            .with_user(&author)
            .with_explicit_name();
        let mut body = ferris_core::Value::from_json(serde_json::json!({
            "user": "ada@example.com",
            "title": "For Ada",
        }));
        body.set("name".to_string(), ferris_core::Value::from("n-1"));
        engine
            .create_owned_by(&as_author, "ada@example.com", body)
            .await
            .unwrap();

        // `Notification` grants `All` nothing, on purpose: its recipient owns
        // it. Grace may not read it by role, by `All`, or by any policy — only
        // by owning it. `list()` answers an empty page rather than an error
        // when every row is unreadable (§C2), so the refusal shows up as the
        // absence of the row and `total` recomputed over what survived.
        let grace = user("grace@example.com", &[]);
        let as_grace = ferris_document::EngineContext::new(crate::core_doctypes::NOTIFICATION, "")
            .with_meta(&meta)
            .with_user(&grace);
        let (leaked, total) = engine
            .list(&as_grace, &ferris_document::ListQuery::default())
            .await
            .unwrap();
        assert_eq!(
            (leaked.len(), total),
            (0, 0),
            "another user's notification leaked"
        );

        let ada = user("ada@example.com", &[]);
        let as_ada = ferris_document::EngineContext::new(crate::core_doctypes::NOTIFICATION, "")
            .with_meta(&meta)
            .with_user(&ada);
        let (docs, total) = engine
            .list(&as_ada, &ferris_document::ListQuery::default())
            .await
            .expect("the recipient owns it, so the owner rule grants read");
        assert_eq!((docs.len(), total), (1, 1));
    }
}
