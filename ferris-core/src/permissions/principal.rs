use serde::{Deserialize, Serialize};

/// Principal (user or role)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Principal {
    pub principal_type: PrincipalType,
    pub identifier: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalType {
    User,
    Role,
    Group,
    Everyone,
}

impl Principal {
    pub fn user(id: impl Into<String>) -> Self {
        Self {
            principal_type: PrincipalType::User,
            identifier: id.into(),
        }
    }

    pub fn role(name: impl Into<String>) -> Self {
        Self {
            principal_type: PrincipalType::Role,
            identifier: name.into(),
        }
    }

    pub fn group(name: impl Into<String>) -> Self {
        Self {
            principal_type: PrincipalType::Group,
            identifier: name.into(),
        }
    }

    pub fn everyone() -> Self {
        Self {
            principal_type: PrincipalType::Everyone,
            identifier: "*".to_string(),
        }
    }
}
