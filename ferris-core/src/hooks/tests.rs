use super::*;
use crate::value::Value;
use std::collections::HashMap;
use std::sync::Arc;

use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn test_hook_execution() {
    let mut registry = HookRegistry::new();
    let mut ctx = HookContext::new("Test".into(), "test-1".into(), "admin".into());

    struct TestDoc {
        fields: HashMap<String, Value>,
    }

    impl DocumentHook for TestDoc {
        fn doctype(&self) -> &str {
            "Test"
        }
        fn name(&self) -> &str {
            "test-1"
        }
        fn get_field(&self, field: &str) -> Option<&Value> {
            self.fields.get(field)
        }
        fn set_field(&mut self, field: String, value: Value) {
            self.fields.insert(field, value);
        }
        fn get_fields(&self) -> &HashMap<String, Value> {
            &self.fields
        }
        fn is_new(&self) -> bool {
            true
        }
        fn docstatus(&self) -> DocStatus {
            DocStatus::Draft
        }
        fn set_docstatus(&mut self, _: DocStatus) {}
        fn owner(&self) -> &str {
            "admin"
        }
        fn set_owner(&mut self, _: String) {}
        fn modified_by(&self) -> &str {
            "admin"
        }
        fn set_modified_by(&mut self, _: String) {}
    }

    let mut doc = TestDoc {
        fields: HashMap::new(),
    };

    registry.register(
        "Test".into(),
        HookPoint::BeforeInsert,
        Box::new(|_ctx, doc| {
            doc.set_field("created_by".into(), Value::from("admin"));
            Box::pin(async { Ok(()) })
        }),
        0,
        None,
    );

    registry
        .execute("Test", HookPoint::BeforeInsert, &mut ctx, &mut doc)
        .await
        .unwrap();
    assert_eq!(doc.get_field("created_by"), Some(&Value::from("admin")));
}

struct CountingDoc {
    fields: HashMap<String, Value>,
}

impl DocumentHook for CountingDoc {
    fn doctype(&self) -> &str {
        "Test"
    }
    fn name(&self) -> &str {
        "test-1"
    }
    fn get_field(&self, field: &str) -> Option<&Value> {
        self.fields.get(field)
    }
    fn set_field(&mut self, field: String, value: Value) {
        self.fields.insert(field, value);
    }
    fn get_fields(&self) -> &HashMap<String, Value> {
        &self.fields
    }
    fn is_new(&self) -> bool {
        true
    }
    fn docstatus(&self) -> DocStatus {
        DocStatus::Draft
    }
    fn set_docstatus(&mut self, _: DocStatus) {}
    fn owner(&self) -> &str {
        "admin"
    }
    fn set_owner(&mut self, _: String) {}
    fn modified_by(&self) -> &str {
        "admin"
    }
    fn set_modified_by(&mut self, _: String) {}
}

/// Hook that only records that it ran.
fn counting(hits: &Arc<AtomicUsize>) -> HookFn {
    let hits = hits.clone();
    Box::new(move |_ctx, _doc| {
        hits.fetch_add(1, Ordering::Relaxed);
        Box::pin(async { Ok(()) })
    })
}

fn counting_ctx() -> (HookContext, CountingDoc) {
    (
        HookContext::new("Test".into(), "test-1".into(), "admin".into()),
        CountingDoc {
            fields: HashMap::new(),
        },
    )
}

#[tokio::test]
async fn re_registering_a_tagged_hook_replaces_it() {
    let hits = Arc::new(AtomicUsize::new(0));
    let mut registry = HookRegistry::new();

    // What the dev server does: re-wire the plugin once per hot reload.
    for _ in 0..3 {
        registry.register_tagged(
            "plugin",
            "Test".into(),
            HookPoint::AfterSave,
            counting(&hits),
            0,
            None,
        );
    }

    let (mut ctx, mut doc) = counting_ctx();
    registry
        .execute("Test", HookPoint::AfterSave, &mut ctx, &mut doc)
        .await
        .unwrap();
    assert_eq!(
        hits.load(Ordering::Relaxed),
        1,
        "the hook must run once, not once per reload"
    );
}

#[tokio::test]
async fn untagged_hooks_survive_a_tagged_registration() {
    // An untagged (`register`) hook is caller-managed; re-wiring the plugin
    // must not evict a hook the app declared itself.
    let tagged = Arc::new(AtomicUsize::new(0));
    let plain = Arc::new(AtomicUsize::new(0));
    let mut registry = HookRegistry::new();

    registry.register(
        "Test".into(),
        HookPoint::AfterSave,
        counting(&plain),
        0,
        None,
    );
    for _ in 0..2 {
        registry.register_tagged(
            "plugin",
            "Test".into(),
            HookPoint::AfterSave,
            counting(&tagged),
            0,
            None,
        );
    }

    let (mut ctx, mut doc) = counting_ctx();
    registry
        .execute("Test", HookPoint::AfterSave, &mut ctx, &mut doc)
        .await
        .unwrap();
    assert_eq!(plain.load(Ordering::Relaxed), 1);
    assert_eq!(tagged.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn distinct_tags_do_not_evict_each_other() {
    let a = Arc::new(AtomicUsize::new(0));
    let b = Arc::new(AtomicUsize::new(0));
    let mut registry = HookRegistry::new();

    registry.register_tagged(
        "plugin",
        "Test".into(),
        HookPoint::AfterSave,
        counting(&a),
        0,
        None,
    );
    registry.register_tagged(
        "other",
        "Test".into(),
        HookPoint::AfterSave,
        counting(&b),
        0,
        None,
    );
    // Re-registering one tag leaves the other alone.
    registry.register_tagged(
        "plugin",
        "Test".into(),
        HookPoint::AfterSave,
        counting(&a),
        0,
        None,
    );

    let (mut ctx, mut doc) = counting_ctx();
    registry
        .execute("Test", HookPoint::AfterSave, &mut ctx, &mut doc)
        .await
        .unwrap();
    assert_eq!(a.load(Ordering::Relaxed), 1);
    assert_eq!(b.load(Ordering::Relaxed), 1);
}

/// The serde spelling and `as_str()` are the same string for every variant.
/// They were not (`beforeinsert` vs `before_insert`), so this pins the pair.
#[test]
fn hook_point_serializes_exactly_as_as_str() {
    const ALL: [HookPoint; 19] = [
        HookPoint::BeforeInsert,
        HookPoint::AfterInsert,
        HookPoint::BeforeSave,
        HookPoint::AfterSave,
        HookPoint::BeforeSubmit,
        HookPoint::OnSubmit,
        HookPoint::BeforeCancel,
        HookPoint::OnCancel,
        HookPoint::BeforeDelete,
        HookPoint::Validate,
        HookPoint::OnUpdate,
        HookPoint::OnTrash,
        HookPoint::BeforeNaming,
        HookPoint::BeforeValidate,
        HookPoint::BeforeUpdateAfterSubmit,
        HookPoint::OnUpdateAfterSubmit,
        HookPoint::OnChange,
        HookPoint::BeforeRename,
        HookPoint::AfterRename,
    ];
    for point in ALL {
        let json = serde_json::to_string(&point).unwrap();
        assert_eq!(json, format!("\"{}\"", point.as_str()));
        let back: HookPoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back, point);
    }
}
