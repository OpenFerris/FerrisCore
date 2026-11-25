//! Persisting the in-code workspace registry as `Workspace` documents.
//!
//! The workspace registry (`ferris_core::workspaces`) is a process-global map
//! built from compiled declarations. This store is its data-driven mirror:
//! every boot writes each declared workspace into `tabWorkspace` as an ordinary
//! document, so the desk is backed by documents the Desk can list and inspect
//! instead of by an in-memory map alone.
//!
//! Two directions, both idempotent:
//!
//! * [`seed_workspaces`] walks the registry and upserts one row per workspace.
//!   This is what the host calls at boot, after `register_workspace_catalog`.
//! * [`load_workspaces`] reads those rows back into [`WorkspaceSpec`]s, which is
//!   what the desk endpoints lay out. A row whose `content` is missing or
//!   unreadable still yields a workspace, rebuilt from its own columns and child
//!   rows.
//!
//! The full ferris `WorkspaceSpec` travels in the `content` field (Frappe's
//! hidden workspace-content column), because the Frappe-shaped columns cannot
//! represent ferris's nested item groups or the grid `folder`; the columns are a
//! projection, `content` is the source — the same split as `DoctypeStore`'s
//! `meta_json`.
#![allow(clippy::result_large_err)] // `Error` is the crate-wide error type; boxing it would leak into every caller.

use ferris_core::workspaces::{WorkspaceItemKind, WorkspaceItemSpec, WorkspaceSpec};
use ferris_core::{Error, Result, SerializationError, Value};
use ferris_db::sql::{Column, Ident, UpsertBuilder, UpsertSet};
use ferris_db::{Db, Stmt};
use ferris_document::{DocumentEngine, EngineContext, ListQuery};
use ferris_schema::DoctypeMeta;
use std::collections::HashMap;

/// The `Workspace` `DocType` this store reads and writes.
pub const WORKSPACE: &str = "Workspace";

/// Framework bookkeeping: the fingerprint the seed wrote for each workspace.
///
/// It is deliberately **not** a `DocType` field: the seed must not add columns
/// to the public `Workspace` document, and the engine rejects a key that the
/// metadata does not declare. Keeping the last seeded fingerprint here is what
/// lets the next boot tell "the declaration changed" (refresh) from "an
/// operator edited the row" (leave it alone) — LOGIC-02.
const SEED_STAMPS_TABLE: &str = "tabWorkspaceSeed";

/// What a [`seed_workspaces`] did.
#[derive(Debug, Default, Clone)]
pub struct WorkspaceSeedReport {
    /// Names of the workspaces the registry declares, in registry order. A row
    /// that already matched the declaration (or an operator's edit) was left
    /// untouched but is still reported here.
    pub seeded: Vec<String>,
    /// `name: error` lines for workspaces that could not be written. A bad
    /// declaration must not stop the boot, so these are reported rather than
    /// returned as an error — the same contract as `seed_singletons`.
    pub warnings: Vec<String>,
}

/// The engine context for a framework workspace write.
///
/// A workspace is framework-owned, not caller-owned: the seed runs before any
/// request exists, as the system with anonymous access allowed. This is the same
/// rationale as `DoctypeStore::context`; the write surface stays role-governed
/// whenever a real user goes through `/api/resource/Workspace` instead.
fn context(meta: Option<&DoctypeMeta>) -> EngineContext<'_> {
    let mut ctx = EngineContext::new(WORKSPACE, "");
    ctx.meta = meta;
    ctx.allow_anonymous(true)
}

/// Stable fingerprint of the parts a seed owns in a `Workspace` row.
///
/// FNV-1a: deterministic across processes, dependency-free, and only used to
/// decide whether a write is needed — not as a security primitive. The fields
/// are the ones the Desk edits (`label`, `content`, placement); `modified`,
/// `owner` and the child-row framing are engine bookkeeping and are excluded so
/// an engine stamp cannot make every boot look like a declaration change.
fn fingerprint(parts: &[&str]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The fingerprint of a document body the seed produced or read back.
fn row_fingerprint(document: &Value) -> Option<String> {
    let label = document.get("label").and_then(Value::as_str)?;
    let content = document.get("content").and_then(Value::as_str)?;
    let module = document
        .get("module")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let icon = document
        .get("icon")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let sequence = document
        .get("sequence_id")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    Some(fingerprint(&[
        label,
        content,
        module,
        icon,
        &sequence.to_string(),
    ]))
}

/// Project one declared workspace into a `Workspace` document body.
fn body(spec: &WorkspaceSpec, sequence: usize) -> Result<Value> {
    let json = serde_json::to_value(spec)
        .map_err(|e| Error::Serialization(SerializationError::Json(e)))?;
    let content = serde_json::to_string(&json)
        .map_err(|e| Error::Serialization(SerializationError::Json(e)))?;

    let mut body = ferris_core::Map::new();
    body.insert("name".to_string(), Value::from(spec.name.clone()));
    body.insert("label".to_string(), Value::from(spec.label.clone()));
    body.insert("title".to_string(), Value::from(spec.label.clone()));
    body.insert("module".to_string(), Value::from(spec.module.clone()));
    body.insert(
        "icon".to_string(),
        Value::from(spec.icon.clone().unwrap_or_default()),
    );
    // A declared workspace is a standard, public Workspace tile (not a plain
    // Link/URL one), so the desk groups it like any shipped workspace.
    body.insert("type".to_string(), Value::from("Workspace"));
    body.insert("standard".to_string(), Value::from(true));
    body.insert("public".to_string(), Value::from(true));
    body.insert("is_hidden".to_string(), Value::from(false));
    body.insert(
        "sequence_id".to_string(),
        Value::from(u32::try_from(sequence).unwrap_or(u32::MAX)),
    );
    body.insert("content".to_string(), Value::from(content));
    body.insert("links".to_string(), link_rows(spec));
    body.insert("shortcuts".to_string(), shortcut_rows(spec));
    body.insert("roles".to_string(), Value::Array(Vec::new()));
    Ok(Value::Object(body))
}

/// The `links` child rows of a declared workspace: the doctype/route links and
/// the `Card Break` rows that open a group (its children follow underneath).
fn link_rows(spec: &WorkspaceSpec) -> Value {
    let mut rows = Vec::new();
    let mut idx: i64 = 0;
    for item in &spec.items {
        push_link_rows(item, &spec.name, &mut idx, &mut rows);
    }
    Value::Array(rows)
}

fn push_link_rows(item: &WorkspaceItemSpec, parent: &str, idx: &mut i64, rows: &mut Vec<Value>) {
    *idx += 1;
    let mut row = child_row(parent, "links", *idx);
    row.insert("label".to_string(), Value::from(item.label.clone()));
    row.insert(
        "icon".to_string(),
        Value::from(item.icon.clone().unwrap_or_default()),
    );

    if !item.children.is_empty() {
        // A group is a link card: a `Card Break` then its members.
        row.insert("type".to_string(), Value::from("Card Break"));
        rows.push(Value::Object(row));
        for child in &item.children {
            push_link_rows(child, parent, idx, rows);
        }
        return;
    }

    row.insert("type".to_string(), Value::from("Link"));
    if item.kind() == WorkspaceItemKind::Doctype {
        row.insert("link_type".to_string(), Value::from("DocType"));
        row.insert(
            "link_to".to_string(),
            Value::from(item.doctype.clone().unwrap_or_default()),
        );
    } else {
        // A ferris route has no Frappe equivalent; `Page` is the closest
        // `link_type`, with the route as the target.
        row.insert("link_type".to_string(), Value::from("Page"));
        row.insert(
            "link_to".to_string(),
            Value::from(item.route.clone().unwrap_or_default()),
        );
    }
    rows.push(Value::Object(row));
}

/// The `shortcuts` child rows of a declared workspace: the external-URL tiles.
///
/// A `${url}`-style link has no home in Frappe's `Workspace Link` (whose
/// `link_type` is only `DocType`/`Page`/`Report`), but `Workspace Shortcut` has
/// a `URL` type and a `url` column, so URLs are projected there. This split is a
/// projection only; `content` is what is read back.
fn shortcut_rows(spec: &WorkspaceSpec) -> Value {
    let rows: Vec<Value> = spec
        .items
        .iter()
        .filter(|item| item.children.is_empty() && item.kind() == WorkspaceItemKind::Url)
        .enumerate()
        .map(|(i, item)| {
            let mut row = child_row(&spec.name, "shortcuts", i64::try_from(i).unwrap_or(0) + 1);
            row.insert("type".to_string(), Value::from("URL"));
            row.insert("label".to_string(), Value::from(item.label.clone()));
            row.insert(
                "icon".to_string(),
                Value::from(item.icon.clone().unwrap_or_default()),
            );
            row.insert(
                "url".to_string(),
                Value::from(item.href.clone().unwrap_or_default()),
            );
            Value::Object(row)
        })
        .collect();
    Value::Array(rows)
}

/// A child row's shared framing (`idx`/`parent`/`parenttype`/`parentfield`).
fn child_row(parent: &str, parentfield: &str, idx: i64) -> ferris_core::Map {
    let mut row = ferris_core::Map::new();
    row.insert("idx".to_string(), Value::from(idx));
    row.insert("parent".to_string(), Value::from(parent));
    row.insert("parenttype".to_string(), Value::from(WORKSPACE));
    row.insert("parentfield".to_string(), Value::from(parentfield));
    row
}

/// Create the seed-stamp table and read the fingerprints written last time.
async fn seed_stamps(pool: &Db) -> Result<HashMap<String, String>> {
    pool.ensure_table(
        SEED_STAMPS_TABLE,
        &[
            Column::new("name", "VARCHAR(255)").not_null().primary_key(),
            Column::new("hash", "VARCHAR(32)").not_null(),
        ],
    )
    .await?;
    let quoted = Ident::new(SEED_STAMPS_TABLE)?.quote(pool.dialect());
    let rows = pool
        .fetch_all(&Stmt::new(
            format!("SELECT name, hash FROM {quoted}"),
            Vec::new(),
        ))
        .await?;
    let mut stamps = HashMap::new();
    for row in rows {
        if let (Ok(name), Ok(hash)) = (row.str("name"), row.str("hash")) {
            stamps.insert(name, hash);
        }
    }
    Ok(stamps)
}

/// Remember the fingerprint this boot wrote for `name`.
async fn save_stamp(pool: &Db, name: &str, hash: &str) -> Result<()> {
    pool.upsert(
        UpsertBuilder::new(pool.dialect(), SEED_STAMPS_TABLE)
            .values(vec![
                ("name", Value::from(name)),
                ("hash", Value::from(hash)),
            ])
            .conflict_on(["name"])
            .on_update("hash", UpsertSet::Value(Value::from(hash)))
            .build()?,
    )
    .await?;
    Ok(())
}

/// Upsert one declared workspace into `tabWorkspace`.
///
/// A row is written only when the declaration actually changed. When the
/// stored fingerprint no longer matches the one this seed wrote, an operator
/// edited the row: their version wins and the declaration is not re-applied
/// (LOGIC-02). `stamps` is `None` when the bookkeeping table could not be
/// opened; the seed then falls back to the old "write when the body differs"
/// behaviour instead of failing.
async fn upsert(
    pool: &Db,
    engine: &DocumentEngine,
    ctx: &EngineContext<'_>,
    spec: &WorkspaceSpec,
    sequence: usize,
    stamps: Option<&HashMap<String, String>>,
) -> Result<()> {
    let body = body(spec, sequence)?;
    let desired = row_fingerprint(&body).unwrap_or_default();
    let existing = match engine.get(ctx, &spec.name).await {
        Ok(document) => Some(document),
        Err(Error::Document(ferris_core::DocumentError::NotFound { .. })) => None,
        Err(error) => return Err(error),
    };

    match existing {
        Some(document) => {
            let current = row_fingerprint(&document);
            if let Some(stamps) = stamps {
                match stamps.get(&spec.name) {
                    // Same declaration, row untouched since the last seed.
                    Some(stamp)
                        if *stamp == desired && current.as_deref() == Some(stamp.as_str()) =>
                    {
                        return Ok(());
                    }
                    // The row changed since the seed wrote it: it is the
                    // operator's now, so a boot must not revert it.
                    Some(stamp) if current.as_deref() != Some(stamp.as_str()) => {
                        return Ok(());
                    }
                    // A row seeded before this table existed and never touched.
                    None if current.as_deref() == Some(desired.as_str()) => {
                        save_stamp(pool, &spec.name, &desired).await?;
                        return Ok(());
                    }
                    _ => {}
                }
            } else if current.as_deref() == Some(desired.as_str()) {
                return Ok(());
            }
            engine.update(ctx, &spec.name, body).await?;
        }
        None => {
            engine.create(ctx, body).await?;
        }
    }
    if stamps.is_some() {
        save_stamp(pool, &spec.name, &desired).await?;
    }
    Ok(())
}

/// Mirror every declared workspace into `tabWorkspace`.
///
/// Idempotent: an existing row is refreshed only when the declaration changed
/// and the row still matches what the previous seed wrote. An operator edit is
/// preserved across boots. Never fails as a whole — a failure for one workspace
/// is collected in [`WorkspaceSeedReport::warnings`] so one bad declaration
/// cannot abort the boot. Must run **after** `register_workspace_catalog()`.
pub async fn seed_workspaces(pool: &Db) -> WorkspaceSeedReport {
    seed_specs(pool, &ferris_core::workspaces::registered_workspaces()).await
}

/// Mirror an explicit list of declared workspaces into `tabWorkspace`.
///
/// The I/O body of [`seed_workspaces`], split out so tests can seed a fixture
/// list without touching the process-global registry (which the desk/panel tests
/// serialise among themselves but cannot share with this module).
async fn seed_specs(pool: &Db, specs: &[WorkspaceSpec]) -> WorkspaceSeedReport {
    let meta = ferris_schema::get_doctype(WORKSPACE);
    let ctx = context(meta.as_deref());
    let engine = DocumentEngine::new(pool.clone());

    let mut report = WorkspaceSeedReport::default();
    let stamps = match seed_stamps(pool).await {
        Ok(stamps) => Some(stamps),
        Err(error) => {
            // The guard is an optimisation and an edit-preservation mechanism;
            // losing it must not stop the mirror from being written.
            report
                .warnings
                .push(format!("workspace seed bookkeeping unavailable: {error}"));
            None
        }
    };
    for (sequence, spec) in specs.iter().enumerate() {
        match upsert(pool, &engine, &ctx, spec, sequence, stamps.as_ref()).await {
            Ok(()) => report.seeded.push(spec.name.clone()),
            Err(e) => report.warnings.push(format!("{}: {e}", spec.name)),
        }
    }
    report
}

/// Read every stored `Workspace` back into its ferris [`WorkspaceSpec`].
///
/// Rows are ordered by `sequence_id` then name, so the layout is stable across
/// boots. A row whose `content` is missing or unreadable is rebuilt from its
/// columns and child rows rather than dropped, so a hand-written workspace still
/// reaches the desk.
///
/// # Errors
///
/// Returns the engine error when `tabWorkspace` cannot be read.
pub async fn load_workspaces(pool: &Db) -> Result<Vec<WorkspaceSpec>> {
    let meta = ferris_schema::get_doctype(WORKSPACE);
    let ctx = context(meta.as_deref());
    let engine = DocumentEngine::new(pool.clone());
    // `limit: None` is explicit: the engine applies its own default page, and
    // the `total` below is what says whether that page was the whole table.
    // LOGIC-03: this used to silently return the first 20 workspaces when an
    // install had more, so later tiles disappeared from the Desk.
    let (mut documents, total) = engine
        .list(
            &ctx,
            &ListQuery {
                limit: None,
                ..Default::default()
            },
        )
        .await?;
    if (documents.len() as u64) < total {
        let limit = u32::try_from(total).unwrap_or(u32::MAX);
        let (all, _) = engine
            .list(
                &ctx,
                &ListQuery {
                    limit: Some(limit),
                    ..Default::default()
                },
            )
            .await?;
        documents = all;
    }

    // `sequence_id` is the declaration index the seed stamped; the name breaks
    // ties (and orders hand-written rows, which have none). This reproduces the
    // registry's declaration order, which the desk layout depends on.
    documents.sort_by(|a, b| {
        let ka = a
            .get("sequence_id")
            .and_then(Value::as_i64)
            .unwrap_or(i64::MAX);
        let kb = b
            .get("sequence_id")
            .and_then(Value::as_i64)
            .unwrap_or(i64::MAX);
        ka.cmp(&kb).then_with(|| {
            let na = a.get("name").and_then(Value::as_str).unwrap_or("");
            let nb = b.get("name").and_then(Value::as_str).unwrap_or("");
            na.cmp(nb)
        })
    });
    Ok(documents.iter().filter_map(spec_of).collect())
}

/// Rebuild one [`WorkspaceSpec`] from a stored row.
///
/// Prefers `content` (lossless). A row without it falls back to the projected
/// columns and the `links`/`shortcuts` child rows.
fn spec_of(doc: &Value) -> Option<WorkspaceSpec> {
    let name = doc.get("name").and_then(Value::as_str)?.to_string();
    if let Some(raw) = doc.get("content").and_then(Value::as_str) {
        let raw = raw.trim();
        if !raw.is_empty() && raw != "[]" {
            if let Ok(mut spec) = serde_json::from_str::<WorkspaceSpec>(raw) {
                if spec.name.is_empty() {
                    spec.name = name;
                }
                return Some(spec);
            }
            tracing::warn!(workspace = %name, "stored `content` is not a valid workspace spec");
        }
    }
    Some(spec_from_columns(doc, &name))
}

/// The column projection of a `Workspace` row, plus its child rows.
fn spec_from_columns(doc: &Value, name: &str) -> WorkspaceSpec {
    let label = doc
        .get("title")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| doc.get("label").and_then(Value::as_str))
        .unwrap_or(name)
        .to_string();
    let module = doc
        .get("module")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let icon = doc
        .get("icon")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    WorkspaceSpec {
        name: name.to_string(),
        label,
        module,
        icon,
        folder: None,
        items: items_from_child_rows(doc),
    }
}

/// Rebuild sidebar items from the `links` and `shortcuts` child rows.
///
/// `Card Break` rows open a group and the `Link` rows that follow join it, which
/// is how the seed wrote them back out.
fn items_from_child_rows(doc: &Value) -> Vec<WorkspaceItemSpec> {
    let mut items: Vec<WorkspaceItemSpec> = Vec::new();
    let mut group: Option<usize> = None;

    for row in doc
        .get("links")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let label = row
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let icon = row
            .get("icon")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        if row.get("type").and_then(Value::as_str) == Some("Card Break") {
            items.push(WorkspaceItemSpec {
                label,
                icon,
                ..Default::default()
            });
            group = Some(items.len() - 1);
            continue;
        }

        let target = row
            .get("link_to")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let item = match row.get("link_type").and_then(Value::as_str) {
            Some("DocType") => WorkspaceItemSpec {
                label,
                icon,
                doctype: Some(target),
                ..Default::default()
            },
            _ => WorkspaceItemSpec {
                label,
                icon,
                route: Some(target),
                ..Default::default()
            },
        };
        match group {
            Some(index) => items[index].children.push(item),
            None => items.push(item),
        }
    }

    for row in doc
        .get("shortcuts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let url = row.get("url").and_then(Value::as_str).unwrap_or_default();
        if url.is_empty() {
            continue;
        }
        items.push(WorkspaceItemSpec {
            label: row
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            icon: row
                .get("icon")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            href: Some(url.to_string()),
            ..Default::default()
        });
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferris_core::workspaces::{WorkspaceItemSpec, WorkspaceSpec};

    fn sample() -> WorkspaceSpec {
        WorkspaceSpec {
            name: "seed-probe".into(),
            label: "Seed Probe".into(),
            module: "Probe".into(),
            icon: Some("Boxes".into()),
            folder: Some("probe-folder".into()),
            items: vec![
                WorkspaceItemSpec::doctype_titled("ToDo", "Tasks").icon("SquareCheck"),
                WorkspaceItemSpec::url("https://wiki.example.test")
                    .label("Docs")
                    .new_tab(),
                WorkspaceItemSpec::group("More", vec![WorkspaceItemSpec::route("/dashboard")]),
            ],
        }
    }

    #[tokio::test]
    async fn a_seeded_workspace_round_trips_through_content() {
        ferris_schema::register_doctype(ferris_schema::DoctypeMeta::new(
            "WorkspaceItemProbe",
            "WorkspaceItemProbe",
        ));
        let db = ferris_db::memory_db().await.unwrap();

        let report = seed_specs(&db, &[sample()]).await;
        assert!(report.warnings.is_empty(), "{report:?}");
        assert!(report.seeded.iter().any(|n| n == "seed-probe"));

        let loaded = load_workspaces(&db).await.unwrap();
        let spec = loaded
            .iter()
            .find(|w| w.name == "seed-probe")
            .expect("the seeded workspace reads back");
        // `content` is lossless, including the grid folder and nested groups.
        assert_eq!(spec.folder.as_deref(), Some("probe-folder"));
        assert_eq!(spec.label, "Seed Probe");
        assert_eq!(spec.items.len(), 3);
        assert_eq!(spec.items[0].doctype.as_deref(), Some("ToDo"));
        assert_eq!(
            spec.items[1].href.as_deref(),
            Some("https://wiki.example.test")
        );
        assert_eq!(spec.items[2].kind(), WorkspaceItemKind::Group);
        assert_eq!(
            spec.items[2].children[0].route.as_deref(),
            Some("/dashboard")
        );
    }

    #[tokio::test]
    async fn load_workspaces_keeps_the_declaration_order() {
        ferris_schema::register_doctype(ferris_schema::DoctypeMeta::new(
            "WorkspaceItemProbe",
            "WorkspaceItemProbe",
        ));
        let db = ferris_db::memory_db().await.unwrap();

        // Deliberately out of alphabetical order: the seed stamps `sequence_id`
        // so the desk reads them back in declaration order, not sorted by name.
        let mut first = sample();
        first.name = "zzz-first".into();
        // `Workspace.label` is unique; the two fixtures need distinct labels.
        first.label = "ZZZ First".into();
        let mut second = sample();
        second.name = "aaa-second".into();
        second.label = "AAA Second".into();
        let report = seed_specs(&db, &[first, second]).await;
        assert!(report.warnings.is_empty(), "{report:?}");

        let loaded = load_workspaces(&db).await.unwrap();
        let names: Vec<&str> = loaded.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, vec!["zzz-first", "aaa-second"]);
    }

    #[tokio::test]
    async fn seeding_is_idempotent() {
        ferris_schema::register_doctype(ferris_schema::DoctypeMeta::new(
            "WorkspaceItemProbe",
            "WorkspaceItemProbe",
        ));
        let db = ferris_db::memory_db().await.unwrap();

        let first = seed_specs(&db, &[sample()]).await;
        let second = seed_specs(&db, &[sample()]).await;
        assert!(first.warnings.is_empty() && second.warnings.is_empty());
        assert_eq!(
            first.seeded.iter().filter(|n| *n == "seed-probe").count(),
            1
        );
        assert_eq!(
            second.seeded.iter().filter(|n| *n == "seed-probe").count(),
            1
        );
    }

    /// LOGIC-02: a boot refreshes a row the seed owns, but never an operator's
    /// edit. The fingerprint of the last seed is what distinguishes them.
    #[tokio::test]
    async fn a_boot_does_not_revert_an_operator_edit() {
        ferris_schema::register_doctype(ferris_schema::DoctypeMeta::new(
            "WorkspaceItemProbe",
            "WorkspaceItemProbe",
        ));
        let db = ferris_db::memory_db().await.unwrap();
        let first = seed_specs(&db, &[sample()]).await;
        assert!(first.warnings.is_empty(), "{first:?}");

        let meta = ferris_schema::get_doctype(WORKSPACE);
        let ctx = context(meta.as_deref());
        let mut edited = sample();
        edited.label = "Operator Label".into();
        DocumentEngine::new(db.clone())
            .update(&ctx, "seed-probe", body(&edited, 0).unwrap())
            .await
            .unwrap();

        let second = seed_specs(&db, &[sample()]).await;
        assert!(second.warnings.is_empty(), "{second:?}");
        let loaded = load_workspaces(&db).await.unwrap();
        let spec = loaded
            .iter()
            .find(|w| w.name == "seed-probe")
            .expect("the edited row survives the boot");
        assert_eq!(
            spec.label, "Operator Label",
            "a boot must not revert an operator's workspace edit"
        );
    }

    /// LOGIC-03: more rows than the engine's default page must all load.
    #[tokio::test]
    async fn load_workspaces_reads_every_row_not_only_the_first_page() {
        ferris_schema::register_doctype(ferris_schema::DoctypeMeta::new(
            "WorkspaceItemProbe",
            "WorkspaceItemProbe",
        ));
        let db = ferris_db::memory_db().await.unwrap();
        let specs: Vec<WorkspaceSpec> = (0..25)
            .map(|index| {
                let mut spec = sample();
                spec.name = format!("page-probe-{index:02}");
                spec.label = format!("Page Probe {index:02}");
                spec
            })
            .collect();
        let report = seed_specs(&db, &specs).await;
        assert!(report.warnings.is_empty(), "{report:?}");

        let loaded = load_workspaces(&db).await.unwrap();
        assert_eq!(loaded.len(), specs.len());
    }

    #[tokio::test]
    async fn a_row_without_content_is_rebuilt_from_its_columns() {
        let db = ferris_db::memory_db().await.unwrap();
        let meta = ferris_schema::get_doctype(WORKSPACE);
        let engine = DocumentEngine::new(db.clone());
        let ctx = context(meta.as_deref());
        let body = Value::from_json(serde_json::json!({
            "name": "hand-written",
            "label": "Hand Written",
            // `Workspace.title` is required by the current meta; a body that
            // omitted it only ever passed while the registry had not been
            // populated yet.
            "title": "Hand Written",
            // `Workspace.type` is required by the current meta; the historical
            // widget row is a Workspace, not a link tile.
            "type": "Workspace",
            "module": "Probe",
            "icon": "FileText",
            "content": "",
            "links": [
                {"idx": 1, "type": "Link", "label": "Tasks", "icon": "SquareCheck",
                 "link_type": "DocType", "link_to": "ToDo"},
                {"idx": 2, "type": "Card Break", "label": "More", "icon": ""},
                {"idx": 3, "type": "Link", "label": "Dashboard", "link_type": "Page",
                 "link_to": "/dashboard"},
            ],
            "shortcuts": [
                {"idx": 1, "type": "URL", "label": "Docs", "url": "https://wiki.example.test"},
            ],
        }));
        engine.create(&ctx, body).await.unwrap();

        let loaded = load_workspaces(&db).await.unwrap();
        let spec = loaded
            .iter()
            .find(|w| w.name == "hand-written")
            .expect("rebuilt from columns");
        assert_eq!(spec.label, "Hand Written");
        assert_eq!(spec.items.len(), 3);
        assert_eq!(spec.items[0].doctype.as_deref(), Some("ToDo"));
        assert_eq!(spec.items[1].kind(), WorkspaceItemKind::Group);
        assert_eq!(
            spec.items[1].children[0].route.as_deref(),
            Some("/dashboard")
        );
        assert_eq!(
            spec.items[2].href.as_deref(),
            Some("https://wiki.example.test")
        );
    }
}
