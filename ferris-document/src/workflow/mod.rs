//! Workflow state machine (Frappe parity).
//!
//! A [`Workflow`] declares the states a doctype can be in and the transitions
//! between them, gated by roles and (optionally) a safe equality condition.
//! Conditions are data, never evaluated code — no `eval`.

use ferris_core::{DocStatus, Error, Result, Value};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

/// Visual style of a workflow state (for the Desk UI).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StateStyle {
    #[default]
    Default,
    Primary,
    Info,
    Success,
    Warning,
    Danger,
    Inverse,
}

/// A workflow state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowState {
    pub state: String,
    /// Human label; falls back to `state` when not set.
    pub label: Option<String>,
    #[serde(default)]
    pub style: StateStyle,
    pub doc_status: DocStatus,
    /// Roles allowed to edit the document while it is in this state.
    pub allow_edit: Vec<String>,
    /// Actions the state advertises in the UI (informational; enforcement comes
    /// from the transitions themselves).
    #[serde(default)]
    pub allowed_actions: Vec<String>,
    /// Role or user field to assign the document to when entering this state.
    pub auto_assign: Option<String>,
}

impl WorkflowState {
    pub fn label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.state)
    }
}

/// A safe equality condition (`field == equals`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowCondition {
    pub field: String,
    pub equals: Value,
}

impl WorkflowCondition {
    pub fn matches(&self, context: &HashMap<String, Value>) -> bool {
        context
            .get(&self.field)
            .map(|v| v == &self.equals)
            .unwrap_or(false)
    }
}

/// A transition between two states.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowTransition {
    /// Optional transition name (defaults to `action`).
    pub name: Option<String>,
    pub action: String,
    pub from_state: String,
    pub to_state: String,
    /// Roles allowed to trigger this action (empty = any role).
    pub allowed_roles: Vec<String>,
    pub condition: Option<WorkflowCondition>,
    /// Fields that must be present (non-null) before the transition is allowed.
    #[serde(default)]
    pub required_fields: Vec<String>,
    /// Whether a notification should be emitted for this transition.
    #[serde(default)]
    pub notify: bool,
}

/// The result of applying a transition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransitionOutcome {
    pub state: String,
    pub doc_status: DocStatus,
}

/// Emitted after a successful transition; listeners can send notifications or
/// run side effects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEvent {
    pub workflow: String,
    pub doctype: String,
    pub action: String,
    pub from_state: String,
    pub to_state: String,
    /// Whether the transition opted into notifications (`notify = true`).
    pub notify: bool,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// A workflow definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub name: String,
    pub doctype: String,
    pub initial_state: String,
    pub states: Vec<WorkflowState>,
    pub transitions: Vec<WorkflowTransition>,
}

impl Workflow {
    pub fn state(&self, name: &str) -> Option<&WorkflowState> {
        self.states.iter().find(|s| s.state == name)
    }

    fn role_allowed(transition: &WorkflowTransition, roles: &[String]) -> bool {
        transition.allowed_roles.is_empty()
            || transition
                .allowed_roles
                .iter()
                .any(|r| roles.iter().any(|ur| ur == r))
    }

    /// Find a transition matching state, action, roles and condition.
    pub fn find_transition(
        &self,
        from_state: &str,
        action: &str,
        roles: &[String],
        context: &HashMap<String, Value>,
    ) -> Option<&WorkflowTransition> {
        self.transitions.iter().find(|t| {
            t.from_state == from_state
                && t.action == action
                && Self::role_allowed(t, roles)
                && t.condition.as_ref().map_or(true, |c| c.matches(context))
        })
    }

    /// Actions available to `roles` from `from_state`.
    pub fn allowed_actions(&self, from_state: &str, roles: &[String]) -> Vec<String> {
        self.transitions
            .iter()
            .filter(|t| t.from_state == from_state && Self::role_allowed(t, roles))
            .map(|t| t.action.clone())
            .collect()
    }

    /// Apply an action, returning the resulting state and docstatus.
    pub fn apply(
        &self,
        from_state: &str,
        action: &str,
        roles: &[String],
        context: &HashMap<String, Value>,
    ) -> Result<TransitionOutcome> {
        self.apply_matching(from_state, roles, context, action, |transition| {
            transition.action == action
        })
    }

    /// Apply the transition from `from_state` to `to_state`.
    ///
    /// An update body names the state it wants (`workflow_state`), not the
    /// action that reaches it, so the validation keyed on the target state needs
    /// its own entry point. Role, condition, required-field and target-state
    /// checks are the same ones [`Self::apply`] runs — both go through
    /// [`Self::apply_matching`] — so an update can never reach a state a
    /// workflow action could not.
    pub fn apply_to(
        &self,
        from_state: &str,
        to_state: &str,
        roles: &[String],
        context: &HashMap<String, Value>,
    ) -> Result<TransitionOutcome> {
        self.apply_matching(from_state, roles, context, to_state, |transition| {
            transition.to_state == to_state
        })
    }

    /// The one transition evaluator: match, then condition, then required
    /// fields, then the target state's `doc_status`.
    fn apply_matching(
        &self,
        from_state: &str,
        roles: &[String],
        context: &HashMap<String, Value>,
        action: &str,
        matches: impl Fn(&WorkflowTransition) -> bool,
    ) -> Result<TransitionOutcome> {
        // A transition may exist but be blocked by its condition.
        let mut blocked_by_condition = false;
        for t in &self.transitions {
            if t.from_state != from_state || !matches(t) || !Self::role_allowed(t, roles) {
                continue;
            }
            if !t.condition.as_ref().map_or(true, |c| c.matches(context)) {
                blocked_by_condition = true;
                continue;
            }
            // Mandatory fields must be present before the transition runs.
            for field in &t.required_fields {
                let present = context.get(field).map(|v| !v.is_null()).unwrap_or(false);
                if !present {
                    return Err(Error::Workflow(ferris_core::WorkflowError::RequiredField(
                        field.clone(),
                    )));
                }
            }
            let state = self.state(&t.to_state).ok_or_else(|| {
                Error::Workflow(ferris_core::WorkflowError::NotFound(t.to_state.clone()))
            })?;
            return Ok(TransitionOutcome {
                state: state.state.clone(),
                doc_status: state.doc_status,
            });
        }

        if blocked_by_condition {
            Err(Error::Workflow(
                ferris_core::WorkflowError::ConditionNotMet(format!(
                    "condition not met for action {action}"
                )),
            ))
        } else {
            Err(Error::Workflow(
                ferris_core::WorkflowError::InvalidTransition {
                    from: from_state.to_string(),
                    to: action.to_string(),
                    action: action.to_string(),
                },
            ))
        }
    }
}

type WorkflowListener = dyn Fn(&WorkflowEvent) + Send + Sync;

mod engine;

#[cfg(test)]
mod tests;

pub use engine::{WorkflowEngine, global_workflow_engine};
