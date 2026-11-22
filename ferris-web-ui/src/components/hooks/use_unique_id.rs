//! Process-unique DOM ids for the primitives that need one.
//!
//! These are **counters**, not randomness: the value only has to be unique
//! within the document so a trigger can point at its overlay. The module used
//! to be called `use_random_*`, which promised entropy the implementation never
//! had (UI11); a caller that needs an unguessable token must not use this.

use std::sync::atomic::{AtomicUsize, Ordering};

const PREFIX: &str = "rust_ui"; // Must NOT contain "/" or "-"

/// A document-unique id with no element prefix.
pub fn use_unique_id() -> String {
    format!("_{PREFIX}_{}", next_id())
}

/// A document-unique id prefixed with the element name (`menubar_rust_ui_7`).
pub fn use_unique_id_for(element: &str) -> String {
    format!("{}_{PREFIX}_{}", element, next_id())
}

/// A `view-transition-name` declaration built from a unique id.
pub fn use_unique_transition_name() -> String {
    format!("view-transition-name: {}", use_unique_id())
}

static COUNTER: AtomicUsize = AtomicUsize::new(1);

fn next_id() -> usize {
    COUNTER.fetch_add(1, Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_prefixed() {
        let first = use_unique_id();
        let second = use_unique_id();
        assert_ne!(first, second);
        assert!(first.starts_with("_rust_ui_"));

        let prefixed = use_unique_id_for("dialog");
        assert!(prefixed.starts_with("dialog_rust_ui_"));
        assert!(!prefixed.contains('-'));
    }
}
