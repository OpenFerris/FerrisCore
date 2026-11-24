//! Behavioural checks on the DocType catalog: registration, visibility and the
//! permission shape of the machine surfaces.
//!
//! They read the **global** registry (via `get_doctype`) rather than the meta
//! builders, so a builder that never reaches `register_doctypes()` fails here
//! instead of silently missing from the install.

use super::*;
use ferris_schema::get_doctype;

#[test]
fn framework_doctypes_register_and_stay_out_of_the_workspace() {
    register_doctypes();
    for meta in framework_doctype_metas() {
        let got = get_doctype(&meta.name).expect("registered");
        assert!(got.hidden, "{} must stay out of the workspace", meta.name);
        assert!(!got.fields.is_empty(), "{} needs fields", meta.name);
    }
}

#[test]
fn data_doctypes_are_visible_and_register() {
    register_doctypes();
    for meta in data_doctype_metas() {
        let got = get_doctype(&meta.name).expect("registered");
        assert!(!got.hidden, "{} should be a workspace item", meta.name);
        assert!(!got.fields.is_empty(), "{} needs fields", meta.name);
        assert!(
            got.module == DATA_MODULE || got.module == DESK_MODULE,
            "{} has unexpected module {}",
            meta.name,
            got.module
        );
    }
}

#[test]
fn only_document_naming_settings_is_a_data_single() {
    let singles: Vec<_> = data_doctype_metas()
        .into_iter()
        .filter(|m| m.is_single)
        .map(|m| m.name)
        .collect();
    assert_eq!(singles, vec!["Document Naming Settings"]);
}

#[test]
fn register_is_idempotent() {
    register_doctypes();
    register_doctypes();
    assert!(get_doctype(NOTIFICATION).is_some());
    assert!(get_doctype(EVENT).is_some());
    assert!(get_doctype(NOTIFICATION_SETTINGS).is_some());
}

/// R-FW-1b: the desired-state DocType is a hidden machine surface whose
/// reads every signed-in agent may pull, while only a `System Manager` may
/// publish or clear it.
#[test]
fn desired_state_reads_are_open_but_writes_are_not() {
    register_doctypes();
    let meta = get_doctype(DESIRED_STATE).expect("registered");
    assert!(
        meta.hidden,
        "the machine API must stay out of the workspace"
    );

    let all = meta
        .permissions
        .iter()
        .find(|p| p.role == ferris_core::permissions::ALL_USER_ROLE)
        .expect("All users may pull desired state");
    assert!(all.read, "the pull half must be open to signed-in agents");
    assert!(
        !all.write && !all.create && !all.delete,
        "the pull grant must not widen into a write grant"
    );

    let manager = meta
        .permissions
        .iter()
        .find(|p| p.role == "System Manager")
        .expect("System Manager owns the publish half");
    assert!(manager.read && manager.write && manager.create && manager.delete);
}

/// Every DocType this crate declares ends up with the standard `tags`
/// field, and `Tag` itself is a registered, visible DocType: the two halves
/// of "tags is a system field and a DocType".
#[test]
fn every_doctype_has_tags_and_tag_is_a_doctype() {
    register_doctypes();
    for meta in doctype_metas() {
        let got = get_doctype(&meta.name).expect("registered");
        let tags = got
            .fields
            .iter()
            .find(|f| f.fieldname == ferris_schema::TAGS_FIELD)
            .unwrap_or_else(|| panic!("{} is missing the standard `tags` field", meta.name));
        assert_eq!(tags.fieldtype, "Data", "{} tags fieldtype", meta.name);
    }

    let tag = get_doctype(TAG).expect("`Tag` is a DocType");
    assert!(!tag.hidden, "`Tag` is a visible Desk item");
    assert!(tag.fields.iter().any(|f| f.fieldname == "tag"));
    assert!(
        tag.fields
            .iter()
            .any(|f| f.fieldname == ferris_schema::TAGS_FIELD)
    );
}
