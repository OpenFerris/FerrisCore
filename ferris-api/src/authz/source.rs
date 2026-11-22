//! Where an install takes its input from, and what it reports afterwards.

use ferris_auth::permissions::SkippedRule;

/// Where the policies come from, beyond the DocType declarations.
#[derive(Debug, Clone, Default)]
pub struct PolicySource {
    /// Operator-authored policies, as a `Policy` document or an array of them.
    ///
    /// This is the escape hatch Frappe does not have and the reason a DocType
    /// does not have to be recompiled to grant a right: Frappe writes `DocPerm`
    /// rows from the Role Permissions Builder, Ferris reads a file. Loaded
    /// *after* the DocType statements, and it can deny them — deny wins
    /// (`permissions.rs:363-366`) — but it cannot be used to bypass a DocType
    /// that declares nothing, because that leaves no statement to override.
    pub policy_json: Option<String>,
    /// Registry entries to leave out entirely.
    ///
    /// Empty means "everything registered". Filters exist for callers that only
    /// want app DocTypes policed and would rather not enumerate framework ones.
    pub exclude: Vec<String>,
}

/// What an install actually did.
///
/// Returned, and logged, rather than trusted: a translation that quietly drops
/// rules is the failure mode this whole module exists to remove, so the count of
/// dropped rules is part of the result and not an afterthought.
#[derive(Debug, Clone, Default)]
pub struct PolicyInstall {
    /// Registry entries visited.
    pub doctypes: usize,
    /// DocTypes that contributed at least one statement.
    pub doctypes_with_rules: usize,
    /// Statements added from DocType declarations.
    pub statements: usize,
    /// Policies loaded from [`PolicySource::policy_json`].
    pub json_policies: usize,
    /// Rules that produced no statement, with the reason.
    pub skipped: Vec<SkippedRule>,
}

impl PolicyInstall {
    /// One line per skipped rule, for a log at warn level.
    pub fn skipped_lines(&self) -> Vec<String> {
        self.skipped
            .iter()
            .map(|s| {
                format!(
                    "role {:?}: {} (rights: {})",
                    s.role,
                    s.reason.as_str(),
                    if s.rights.is_empty() {
                        "none".to_string()
                    } else {
                        s.rights
                            .iter()
                            .map(|r| r.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                )
            })
            .collect()
    }
}
