//! Tests for [`crate::authz`], split by concern.
//!
//! `support` holds the fixtures and the process-wide engine guard; every test
//! module shares them.
//!
//! `builder` sorts before the other test modules on purpose. The schema
//! registry is process-wide and has no deregister, so a fixture that produces
//! a skipped rule stays visible to every later install in this binary. The
//! Role Permissions Builder test asserts on the *number* of skipped rules in
//! its install, exactly as it did when every test lived in one module whose
//! name sorted it first; keeping it first preserves that ordering.

mod builder;
mod engine;
mod install;
mod operator;
mod support;
