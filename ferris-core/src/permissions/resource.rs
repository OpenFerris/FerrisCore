use serde::{Deserialize, Serialize};

/// Resource identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Resource {
    pub resource_type: String,
    pub name: String,
    pub owner: Option<String>,
}

impl Resource {
    pub fn new(resource_type: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            resource_type: resource_type.into(),
            name: name.into(),
            owner: None,
        }
    }

    pub fn with_owner(mut self, owner: impl Into<String>) -> Self {
        self.owner = Some(owner.into());
        self
    }
}

/// `type:name`, or `type:name:owner` for a row-level resource — the string form
/// used in audit messages and error text.
impl std::fmt::Display for Resource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.owner {
            Some(owner) => write!(f, "{}:{}:{}", self.resource_type, self.name, owner),
            None => write!(f, "{}:{}", self.resource_type, self.name),
        }
    }
}
