//! User model and management.
//!
//! The identity type lives in [`ferris_core::user`], at the base of the crate
//! graph, so the document engine can authorise reads and writes against a user
//! without depending on `ferris-auth` (that would be a crate cycle once this
//! crate uses the engine for its own stores). This module re-exports the core
//! declaration so `ferris_auth::User`, `ferris_auth::UserType` and the
//! `CreateUserInput`/`UpdateUserInput`/`UserFilters` DTOs keep their historical
//! path.
pub use ferris_core::user::{
    CreateUserInput, UpdateUserInput, User, UserFilters, UserRole, UserType,
};
