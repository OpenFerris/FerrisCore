use ferris_core::PermissionRule;

/// Builder for a [`PermissionRule`] (`DocPerm` row).
///
/// Starts all-false (the safe zero) and turns on only the named rights:
///
/// ```ignore
/// RolePermission::new("System Manager").read().write().create().delete()
/// ```
///
/// Convert it with [`RolePermission::build`] or let it convert implicitly into
/// [`DoctypeMeta::permission`].
#[derive(Debug, Clone)]
pub struct RolePermission(PermissionRule);

impl RolePermission {
    /// A rule for `role` granting nothing yet.
    #[must_use]
    pub fn new(role: impl Into<String>) -> Self {
        Self(PermissionRule {
            role: role.into(),
            custom_permissions: Vec::new(),
            ..PermissionRule::default()
        })
    }

    /// Grant read.
    #[must_use]
    pub fn read(mut self) -> Self {
        self.0.read = true;
        self
    }

    /// Grant write.
    #[must_use]
    pub fn write(mut self) -> Self {
        self.0.write = true;
        self
    }

    /// Grant create.
    #[must_use]
    pub fn create(mut self) -> Self {
        self.0.create = true;
        self
    }

    /// Grant delete.
    #[must_use]
    pub fn delete(mut self) -> Self {
        self.0.delete = true;
        self
    }

    /// Grant submit.
    #[must_use]
    pub fn submit(mut self) -> Self {
        self.0.submit = true;
        self
    }

    /// Grant cancel.
    #[must_use]
    pub fn cancel(mut self) -> Self {
        self.0.cancel = true;
        self
    }

    /// Grant amend.
    #[must_use]
    pub fn amend(mut self) -> Self {
        self.0.amend = true;
        self
    }

    /// Grant print.
    #[must_use]
    pub fn print(mut self) -> Self {
        self.0.print = true;
        self
    }

    /// Grant email.
    #[must_use]
    pub fn email(mut self) -> Self {
        self.0.email = true;
        self
    }

    /// Grant share.
    #[must_use]
    pub fn share(mut self) -> Self {
        self.0.share = true;
        self
    }

    /// Grant report.
    #[must_use]
    pub fn report(mut self) -> Self {
        self.0.report = true;
        self
    }

    /// Grant import.
    #[must_use]
    pub fn import(mut self) -> Self {
        self.0.import = true;
        self
    }

    /// Grant export.
    #[must_use]
    pub fn export(mut self) -> Self {
        self.0.export = true;
        self
    }

    /// Limit the rule to documents the caller owns.
    #[must_use]
    pub fn if_owner(mut self) -> Self {
        self.0.if_owner = true;
        self
    }

    /// Set the permission level.
    #[must_use]
    pub fn permlevel(mut self, level: u8) -> Self {
        self.0.permlevel = level;
        self
    }

    /// Finish the builder.
    #[must_use]
    pub fn build(self) -> PermissionRule {
        self.0
    }
}

impl From<RolePermission> for PermissionRule {
    fn from(value: RolePermission) -> Self {
        value.0
    }
}
