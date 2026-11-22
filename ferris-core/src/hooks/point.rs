use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocStatus {
    Draft = 0,
    Submitted = 1,
    Cancelled = 2,
}

impl Default for DocStatus {
    fn default() -> Self {
        DocStatus::Draft
    }
}

impl DocStatus {
    /// Whether a document may move from `self` to `target`.
    ///
    /// Draft → Submitted → Cancelled. A cancelled document is amended into a
    /// new draft, never reopened in place; that is why `Cancelled → Draft` is
    /// absent here (DOC-02).
    pub fn can_transition_to(self, target: DocStatus) -> bool {
        matches!(
            (self, target),
            (DocStatus::Draft, DocStatus::Submitted) | (DocStatus::Submitted, DocStatus::Cancelled)
        )
    }

    /// Whether the document is still a draft (`docstatus = 0`).
    #[must_use]
    pub fn is_draft(self) -> bool {
        matches!(self, DocStatus::Draft)
    }

    /// Whether the document has been submitted (`docstatus = 1`).
    #[must_use]
    pub fn is_submitted(self) -> bool {
        matches!(self, DocStatus::Submitted)
    }

    /// Whether the document has been cancelled (`docstatus = 2`).
    #[must_use]
    pub fn is_cancelled(self) -> bool {
        matches!(self, DocStatus::Cancelled)
    }

    /// The integer the database stores.
    ///
    /// Frappe persists `docstatus` as `0` (draft), `1` (submitted) and `2`
    /// (cancelled) — the same values as the enum discriminants
    /// (<https://docs.frappe.io/framework/doctypes/docstatus>).
    #[must_use]
    pub fn as_i64(self) -> i64 {
        self as i64
    }

    /// Read a stored `docstatus`, returning `None` for a value outside 0/1/2
    /// instead of silently mapping it onto a real state.
    #[must_use]
    pub fn from_i64(value: i64) -> Option<Self> {
        match value {
            0 => Some(DocStatus::Draft),
            1 => Some(DocStatus::Submitted),
            2 => Some(DocStatus::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookPoint {
    BeforeInsert,
    AfterInsert,
    BeforeSave,
    AfterSave,
    BeforeSubmit,
    OnSubmit,
    BeforeCancel,
    OnCancel,
    BeforeDelete,
    Validate,
    OnUpdate,
    OnTrash,
    /// Runs before the engine mints the document name, so a controller may
    /// influence naming.
    BeforeNaming,
    /// Runs before validation (Frappe's controller order: before_naming,
    /// before_validate, validate, before_save).
    BeforeValidate,
    /// Runs after a submitted document is updated (Frappe's
    /// `before_update_after_submit`).
    BeforeUpdateAfterSubmit,
    /// Runs after a submitted document was updated.
    OnUpdateAfterSubmit,
    /// Runs whenever a field changes (Frappe's `on_change` controller hook).
    OnChange,
    /// Runs before the document is renamed.
    BeforeRename,
    /// Runs after the document was renamed.
    AfterRename,
}

impl HookPoint {
    /// The snake_case wire spelling.
    ///
    /// This is the spelling serde produces too (`rename_all = "snake_case"`).
    /// The enum used to serialize as `beforeinsert` while this returned
    /// `before_insert`, i.e. two spellings for one variant, so anything that
    /// round-tripped through both disagreed. The test
    /// `hook_point_serializes_exactly_as_as_str` keeps them equal.
    pub fn as_str(&self) -> &'static str {
        match self {
            HookPoint::BeforeInsert => "before_insert",
            HookPoint::AfterInsert => "after_insert",
            HookPoint::BeforeSave => "before_save",
            HookPoint::AfterSave => "after_save",
            HookPoint::BeforeSubmit => "before_submit",
            HookPoint::OnSubmit => "on_submit",
            HookPoint::BeforeCancel => "before_cancel",
            HookPoint::OnCancel => "on_cancel",
            HookPoint::BeforeDelete => "before_delete",
            HookPoint::Validate => "validate",
            HookPoint::OnUpdate => "on_update",
            HookPoint::OnTrash => "on_trash",
            HookPoint::BeforeNaming => "before_naming",
            HookPoint::BeforeValidate => "before_validate",
            HookPoint::BeforeUpdateAfterSubmit => "before_update_after_submit",
            HookPoint::OnUpdateAfterSubmit => "on_update_after_submit",
            HookPoint::OnChange => "on_change",
            HookPoint::BeforeRename => "before_rename",
            HookPoint::AfterRename => "after_rename",
        }
    }
}
