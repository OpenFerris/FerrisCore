//! Desk endpoints for document engagement: toggling a like and reading the
//! like/comment state of a page of documents.
//!
//! Both live under `/api/engagement/{doctype}` rather than
//! `/api/resource/{doctype}/…` because they act on side tables keyed by
//! `(doctype, docname, user)`, not on the document itself: there is no document
//! to address and no field to read or write.

use std::collections::HashSet;
use std::hash::BuildHasher;
use std::sync::Arc;

use axum::Json;
use axum::extract::{FromRef, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ferris_auth::middleware::AuthenticatedUser;
use ferris_db::Db;
use ferris_document::{CommentStore, LikeStore};
use serde::Deserialize;

use crate::ApiState;

/// Which documents of a page the caller cares about.
///
/// A JSON body rather than a query string because a docname may contain a comma,
/// so joining names into one `names=a,b` value would be ambiguous — and
/// `serde_urlencoded` cannot turn repeated `names` keys into a list at all.
#[derive(Debug, Default, Deserialize)]
pub struct PageQuery {
    /// Docnames, normally the rows the list just loaded.
    ///
    /// Required by the handler (API-08): an absent or empty list is rejected,
    /// because "no names" used to mean "the whole `DocType`".
    #[serde(default)]
    names: Option<Vec<String>>,
}

impl PageQuery {
    fn names(&self) -> HashSet<String> {
        self.names
            .iter()
            .flatten()
            .filter(|name| !name.is_empty())
            .cloned()
            .collect()
    }
}

/// Keep only the entries the caller asked about.
///
/// Generic over the hasher so callers are not forced through the default one
/// (clippy `implicit_hasher`).
fn restrict<S: BuildHasher>(
    map: std::collections::HashMap<String, u64>,
    wanted: &HashSet<String, S>,
) -> std::collections::HashMap<String, u64> {
    map.into_iter()
        .filter(|(name, _)| wanted.contains(name))
        .collect()
}

/// The docnames of `names` the caller may actually `Read`.
///
/// The counts are per document, but they are read from side tables that carry
/// no permission of their own, so answering for a docname the caller cannot
/// read leaks that it exists and how active it is (API-08). Each name goes
/// through `ensure_readable`, which delegates to the engine's one permission
/// policy; a name that fails (denied, gone, or a metadata error) is dropped
/// rather than answered — fail closed, because the alternative is an existence
/// oracle with counts attached.
async fn readable_names(pool: &Db, doctype: &str, names: HashSet<String>) -> HashSet<String> {
    let mut readable = HashSet::with_capacity(names.len());
    for name in names {
        match crate::store::ensure_readable(pool, doctype, &name).await {
            Ok(()) => {
                readable.insert(name);
            }
            Err(error) => {
                tracing::debug!(%error, %name, %doctype, "engagement: skipping an unreadable docname");
            }
        }
    }
    readable
}

/// `POST /api/engagement/{doctype}` — likes and comment counts for one page.
///
/// The list view needs three things for a page of rows (did I like this, how
/// many likes, how many comments) and asking for them per document would be one
/// request per row, so they come back together here.
///
/// `names` is **required**: without it there is no page to answer for, and the
/// old behaviour — aggregating the whole `DocType` — handed every authenticated
/// caller the activity of documents they cannot read (API-08). A body without
/// names is a `400`; names the caller cannot read are filtered out, one
/// document at a time.
pub async fn engagement<S>(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Path(doctype): Path<String>,
    Json(page): Json<PageQuery>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    if let Err(e) = crate::store::validate_doctype(&doctype) {
        return crate::router::error_response(e);
    }

    let wanted = page.names();
    if wanted.is_empty() {
        return crate::router::error_response(ferris_core::Error::Validation(
            ferris_core::ValidationError::Invalid(
                "a page of document names is required; this endpoint does not answer for a whole DocType"
                    .to_string(),
            ),
        ));
    }
    let readable = readable_names(&state.pool, &doctype, wanted).await;

    match read_page(&state.pool, &doctype, &user.0.id, readable).await {
        Ok(page) => Json(page).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// `POST /api/engagement/{doctype}/{name}/like` — like the document, or unlike
/// it when already liked. The response is the state that resulted, so the
/// caller does not have to guess whether the toggle landed.
pub async fn toggle_like<S>(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Path((doctype, name)): Path<(String, String)>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    if let Err(e) = crate::store::validate_doctype(&doctype) {
        return crate::router::error_response(e);
    }
    if let Err(e) = crate::store::validate_name(&name) {
        return crate::router::error_response(e);
    }
    if let Err(e) = crate::store::ensure_readable(&state.pool, &doctype, &name).await {
        return crate::router::error_response(e);
    }

    match LikeStore::new(state.pool.clone())
        .toggle(&doctype, &name, &user.0.id)
        .await
    {
        Ok(like) => Json(like).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// `POST /api/engagement/{doctype}/{name}/comments` — post a comment, or a
/// reply when the body carries a `parent`.
///
/// The body is validated in [`CommentStore`], so every way of commenting
/// (empty body, a `parent` that is not a comment of this document) rejects the
/// same way.
pub async fn add_comment<S>(
    State(state): State<Arc<ApiState>>,
    user: AuthenticatedUser,
    Path((doctype, name)): Path<(String, String)>,
    Json(body): Json<CommentBody>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    if let Err(e) = crate::store::validate_doctype(&doctype) {
        return crate::router::error_response(e);
    }
    if let Err(e) = crate::store::validate_name(&name) {
        return crate::router::error_response(e);
    }
    if let Err(e) = crate::store::ensure_readable(&state.pool, &doctype, &name).await {
        return crate::router::error_response(e);
    }

    match CommentStore::new(state.pool.clone())
        .add(
            &doctype,
            &name,
            &user.0.id,
            &body.body,
            body.parent.as_deref(),
        )
        .await
    {
        Ok(comment) => (StatusCode::CREATED, Json(comment)).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// `GET /api/engagement/{doctype}/{name}/comments` — the document's comments,
/// oldest first, each carrying its `parent` so the panel can thread replies.
pub async fn list_comments<S>(
    State(state): State<Arc<ApiState>>,
    _user: AuthenticatedUser,
    Path((doctype, name)): Path<(String, String)>,
) -> Response
where
    Arc<ApiState>: FromRef<S>,
    ferris_auth::AuthState: FromRef<S>,
    S: Send + Sync,
{
    if let Err(e) = crate::store::validate_doctype(&doctype) {
        return crate::router::error_response(e);
    }
    if let Err(e) = crate::store::validate_name(&name) {
        return crate::router::error_response(e);
    }
    if let Err(e) = crate::store::ensure_readable(&state.pool, &doctype, &name).await {
        return crate::router::error_response(e);
    }

    match CommentStore::new(state.pool.clone())
        .list(&doctype, &name)
        .await
    {
        Ok(comments) => Json(comments).into_response(),
        Err(e) => crate::router::error_response(e),
    }
}

/// Comment body as sent by the Desk. `parent` (the id of the comment being
/// replied to) is optional and defaults to a top-level comment.
#[derive(Debug, Default, Deserialize)]
pub struct CommentBody {
    #[serde(default)]
    body: String,
    #[serde(default)]
    parent: Option<String>,
}

/// The caller's liked docnames plus the like and comment counts of `wanted`.
///
/// An empty `wanted` answers with an empty page: there is no page context, so
/// there is nothing to answer for. Reading the whole `DocType` here is what the
/// aggregate endpoint used to do, and it is exactly what API-08 closed — a
/// caller had counts for every document, readable or not, just by sending no
/// names.
///
/// # Errors
///
/// Returns the storage error of creating a side table or reading its rows.
pub async fn read_page<S: BuildHasher>(
    pool: &Db,
    doctype: &str,
    user: &str,
    wanted: HashSet<String, S>,
) -> ferris_core::Result<ferris_document::EngagementPage> {
    if wanted.is_empty() {
        return Ok(ferris_document::EngagementPage {
            liked: HashSet::new(),
            like_counts: std::collections::HashMap::new(),
            comment_counts: std::collections::HashMap::new(),
        });
    }
    let likes = LikeStore::new(pool.clone());
    let liked = likes.liked_names(doctype, user).await?;
    let like_counts = restrict(likes.like_counts(doctype).await?, &wanted);
    let comment_counts = restrict(
        CommentStore::new(pool.clone()).counts(doctype).await?,
        &wanted,
    );

    Ok(ferris_document::EngagementPage {
        // A docname outside `wanted` is not on screen, so it must not show up
        // as a highlighted row either.
        //
        // `liked` uses the default hasher while `wanted` may not, so the
        // intersection is computed by hand rather than with
        // `HashSet::intersection`, which requires both hashers to match.
        liked: liked
            .into_iter()
            .filter(|name| wanted.contains(name))
            .collect(),
        like_counts,
        comment_counts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> Db {
        ferris_db::memory_db().await.unwrap()
    }

    fn wanted(names: &[&str]) -> HashSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    /// The list asks for one page; the response carries that page and nothing
    /// else, so a row the user cannot see never lights up.
    #[tokio::test]
    async fn page_state_covers_the_requested_rows_only() {
        let pool = pool().await;
        let likes = LikeStore::new(pool.clone());
        let comments = CommentStore::new(pool.clone());

        likes
            .toggle("ToDo", "TD-1", "ada@example.com")
            .await
            .unwrap();
        likes
            .toggle("ToDo", "TD-1", "grace@example.com")
            .await
            .unwrap();
        likes
            .toggle("ToDo", "TD-2", "ada@example.com")
            .await
            .unwrap();
        likes
            .toggle("ToDo", "TD-9", "ada@example.com")
            .await
            .unwrap();
        comments
            .add("ToDo", "TD-1", "ada@example.com", "hola", None)
            .await
            .unwrap();
        comments
            .add("ToDo", "TD-1", "grace@example.com", "otro", None)
            .await
            .unwrap();
        comments
            .add("ToDo", "TD-9", "ada@example.com", "fuera de pagina", None)
            .await
            .unwrap();

        let page = read_page(&pool, "ToDo", "ada@example.com", wanted(&["TD-1", "TD-2"]))
            .await
            .unwrap();

        assert_eq!(page.liked, wanted(&["TD-1", "TD-2"]));
        assert_eq!(page.like_counts.get("TD-1"), Some(&2));
        assert_eq!(page.like_counts.get("TD-2"), Some(&1));
        assert_eq!(page.comment_counts.get("TD-1"), Some(&2));
        assert!(!page.comment_counts.contains_key("TD-9"));
        assert!(!page.like_counts.contains_key("TD-9"));
    }

    /// The body is what the Desk actually sends, so the names are parsed from
    /// the same JSON the route gets rather than from a hand-built value.
    #[test]
    fn query_names_come_from_the_request_body() {
        let parse = |body: serde_json::Value| {
            serde_json::from_value::<PageQuery>(body)
                .expect("body parses")
                .names()
        };

        assert_eq!(
            parse(serde_json::json!({ "names": ["TD-1", "TD-2"] })),
            wanted(&["TD-1", "TD-2"])
        );
        // A single name is still a list of one.
        assert_eq!(
            parse(serde_json::json!({ "names": ["TD-1"] })),
            wanted(&["TD-1"])
        );
        // Docnames keep whatever punctuation they carry, commas included.
        assert_eq!(
            parse(serde_json::json!({ "names": ["TD,1"] })),
            wanted(&["TD,1"])
        );
        // Blank names are dropped rather than kept as a name that matches
        // nothing; an absent or empty list leaves the page empty, and the
        // handler refuses it instead of aggregating the whole DocType.
        assert_eq!(
            parse(serde_json::json!({ "names": ["TD-1", ""] })),
            wanted(&["TD-1"])
        );
        assert!(parse(serde_json::json!({})).is_empty());
        assert!(parse(serde_json::json!({ "names": [] })).is_empty());
    }

    /// Without a page context there is nothing to answer for: the endpoint used
    /// to return the whole `DocType`'s counts here, which is the leak API-08
    /// closed. `read_page` is the shared helper the SSR bridge also calls, so
    /// the refusal lives here and not only in the HTTP handler.
    #[tokio::test]
    async fn an_empty_page_reads_nothing() {
        let pool = pool().await;
        let likes = LikeStore::new(pool.clone());
        likes
            .toggle("ToDo", "TD-1", "ada@example.com")
            .await
            .unwrap();
        likes
            .toggle("ToDo", "TD-2", "ada@example.com")
            .await
            .unwrap();

        let page = read_page(&pool, "ToDo", "ada@example.com", HashSet::new())
            .await
            .unwrap();

        assert!(page.liked.is_empty());
        assert!(page.like_counts.is_empty());
        assert!(page.comment_counts.is_empty());
    }

    /// API-08: a document the caller cannot `Read` is not part of the page,
    /// even when the caller names it explicitly. The like/comment rows exist —
    /// the answer is what must not carry them — and a readability check is what
    /// decides, because the side tables have no permission of their own.
    #[tokio::test]
    async fn a_document_outside_the_read_scope_is_not_counted() {
        use ferris_document::DocumentEngine;

        let pool = pool().await;
        ferris_schema::register_doctype(ferris_schema::doctypes::widget_meta());
        ferris_schema::register_doctype(ferris_schema::doctypes::widget_param_meta());
        // `Widget` is `System Manager`-only, so a plain signed-in caller is
        // denied and an administrator is not.
        let engine = DocumentEngine::new(pool.clone());
        let meta = ferris_schema::get_doctype(crate::widgets::WIDGET);
        let ctx = crate::service::service_context(
            crate::widgets::WIDGET,
            meta.as_deref(),
            "engagement-test@ferris.local",
            "Engagement Test",
        );
        let created = engine
            .create(
                &ctx,
                ferris_core::Value::from_json(serde_json::json!({
                    "app": "app",
                    "widget_name": "kpi",
                    "enabled": true,
                })),
            )
            .await
            .unwrap();
        let name = created
            .get("name")
            .and_then(|value| value.as_str())
            .expect("the row is named")
            .to_string();
        LikeStore::new(pool.clone())
            .toggle(crate::widgets::WIDGET, &name, "grace@example.com")
            .await
            .unwrap();

        let plain =
            ferris_auth::User::system_user("grace@example.com".to_string(), "Grace".to_string());
        let names = wanted(&[name.as_str()]);
        let (readable, page) = ferris_document::with_user(Some(plain), async {
            let visible = readable_names(&pool, crate::widgets::WIDGET, names.clone()).await;
            let page = read_page(
                &pool,
                crate::widgets::WIDGET,
                "grace@example.com",
                visible.clone(),
            )
            .await
            .unwrap();
            (visible, page)
        })
        .await;
        assert!(readable.is_empty(), "a foreign document is not readable");
        assert!(page.like_counts.is_empty());
        assert!(page.liked.is_empty());

        // The same name, read by an administrator, is answered: the gate is the
        // permission, not the endpoint.
        let mut admin =
            ferris_auth::User::system_user("admin@local".to_string(), "Admin".to_string());
        admin.roles = vec!["System Manager".to_string()];
        let readable = ferris_document::with_user(Some(admin), async {
            readable_names(&pool, crate::widgets::WIDGET, wanted(&[name.as_str()])).await
        })
        .await;
        assert_eq!(readable, wanted(&[name.as_str()]));
        let page = read_page(&pool, crate::widgets::WIDGET, "grace@example.com", readable)
            .await
            .unwrap();
        assert_eq!(page.like_counts.get(&name), Some(&1));
    }
}
