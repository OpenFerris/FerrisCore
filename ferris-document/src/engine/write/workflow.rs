use super::super::*;
use ferris_core::hooks::DocStatus;
use ferris_core::permissions::{ALL_USER_ROLE, SYSTEM_MANAGER_ROLE};
use ferris_core::{Error, Result, Value};

/// A workflow move the engine has validated but not yet persisted.
///
/// The state the document left and the state it lands in travel together
/// because persistence and notification both need the pair: the row stores
/// [`Self::outcome`]'s `state`, and the event emitted after the write names the
/// edge, not just its destination.
#[derive(Debug)]
pub(super) struct WorkflowMove {
    pub(super) from_state: String,
    pub(super) outcome: crate::workflow::TransitionOutcome,
}

/// The roles the caller acts with (empty for an anonymous context).
fn caller_roles(ctx: &EngineContext<'_>) -> Vec<String> {
    ctx.user
        .as_ref()
        .map(|user| user.roles.clone())
        .unwrap_or_default()
}

/// The state a document is in, or the workflow's entry state when it has never
/// left it.
fn current_state<'a>(
    workflow: &'a crate::workflow::Workflow,
    stored: &'a HashMap<String, Value>,
) -> &'a str {
    stored
        .get("workflow_state")
        .and_then(Value::as_str)
        .filter(|state| !state.is_empty())
        .unwrap_or(workflow.initial_state.as_str())
}

/// Gate a `docstatus` transition behind a registered workflow, when the doctype
/// has one (DOC6).
///
/// Without this, a `#[workflow]` declaration was validated and stored but never
/// consulted: `set_docstatus` drove the `DocStatus` machine through
/// `DocStatus::can_transition_to` alone. Now, when
/// [`crate::workflow::global_workflow_engine`] holds a workflow for the doctype,
/// the transition must also be one the workflow allows for the caller's roles
/// and the document's fields, and the state it lands in must carry the target
/// `doc_status` — a workflow cannot be used to skip straight past its own states.
///
/// The workflow's source state is the document's `workflow_state` field when it
/// has one (Frappe stores it on the document), falling back to the workflow's
/// `initial_state`. A doctype with **no** registered workflow is untouched: the
/// bare `DocStatus` machine remains the whole rule, exactly as before.
///
/// Returns the validated outcome so the caller can persist `outcome.state`
/// (DOC-09). The current `set_docstatus` call site still discards it because
/// that file is outside this patch's scope; the returned value is exactly what
/// a one-line change there needs.
pub(crate) fn enforce_workflow(
    ctx: &EngineContext<'_>,
    document: &Value,
    current: DocStatus,
    target: DocStatus,
) -> Result<Option<crate::workflow::TransitionOutcome>> {
    let Some(workflow) = crate::workflow::global_workflow_engine().get(&ctx.doctype) else {
        return Ok(None);
    };
    let action = match target {
        DocStatus::Submitted => "Submit",
        DocStatus::Cancelled => "Cancel",
        DocStatus::Draft => "Amend",
    };
    let from_state = document
        .get("workflow_state")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(workflow.initial_state.as_str());
    let roles = caller_roles(ctx);
    let context: HashMap<String, Value> = document
        .as_object()
        .map(|map| map.clone().into_iter().collect())
        .unwrap_or_default();
    let outcome = workflow.apply(from_state, action, &roles, &context)?;
    // `apply` matches on action and source state; the outcome's `doc_status` is
    // the workflow's own statement of where that lands. A mismatch means the
    // workflow's target state does not describe the transition the caller asked
    // for, so the two models disagree and the write must not proceed.
    if outcome.doc_status != target {
        return Err(Error::Workflow(
            ferris_core::WorkflowError::InvalidTransition {
                from: format!("{current:?}"),
                to: format!("{target:?}"),
                action: action.to_string(),
            },
        ));
    }
    Ok(Some(outcome))
}

/// Validate a `workflow_state` carried by an **update** body.
///
/// A plain `PUT` used to be able to set the field the workflow reads and writes
/// — the body was merged as document data, so a caller could move a document to
/// any state without the transition, the roles or the required fields the
/// workflow declares (DOC-09). Now the incoming state is treated as a request
/// to take a declared transition *from the stored state*, and the persisted
/// value is [`TransitionOutcome::state`](crate::workflow::TransitionOutcome),
/// never the raw string.
///
/// `Ok(None)` means the body does not name the field at all, which leaves
/// `workflow_state` untouched. A doctype with no registered workflow cannot
/// accept the key: there is no state machine to validate it against.
pub(super) fn resolve_workflow_update(
    ctx: &EngineContext<'_>,
    stored: &HashMap<String, Value>,
    incoming: &HashMap<String, Value>,
) -> Result<Option<WorkflowMove>> {
    let Some(requested) = incoming.get("workflow_state") else {
        return Ok(None);
    };
    let Some(requested) = requested
        .as_str()
        .map(str::trim)
        .filter(|state| !state.is_empty())
    else {
        return Err(Error::Validation(ValidationError::Invalid(
            "`workflow_state` must name a workflow state".to_string(),
        )));
    };
    let Some(workflow) = crate::workflow::global_workflow_engine().get(&ctx.doctype) else {
        return Err(Error::Validation(ValidationError::Invalid(format!(
            "`{}` has no workflow, so `workflow_state` cannot be set",
            ctx.doctype
        ))));
    };
    let from_state = current_state(&workflow, stored);
    if from_state == requested {
        // Re-sending the current state is the no-op a round-tripped document
        // produces; it must not require a self-transition.
        let state = workflow.state(requested).ok_or_else(|| {
            Error::Workflow(ferris_core::WorkflowError::NotFound(requested.to_string()))
        })?;
        return Ok(Some(WorkflowMove {
            from_state: from_state.to_string(),
            outcome: crate::workflow::TransitionOutcome {
                state: state.state.clone(),
                doc_status: state.doc_status,
            },
        }));
    }
    // Required fields and conditions may be satisfied by the same payload, so
    // they are evaluated against the document as it will be.
    let mut context = stored.clone();
    context.extend(
        incoming
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    let outcome = workflow.apply_to(from_state, requested, &caller_roles(ctx), &context)?;
    Ok(Some(WorkflowMove {
        from_state: from_state.to_string(),
        outcome,
    }))
}

/// Enforce the source state's `allow_edit` role list on an ordinary update.
///
/// Frappe's `Workflow State.allow_edit` names the roles that may edit the
/// document while it is in that state. Ferris stored the list and never read
/// it, so any caller with `Write` could edit a document a workflow had locked
/// to another role. An empty list means "no state-level restriction" and keeps
/// the role rules as the whole policy.
///
/// A caller outside the list may still move the document through the workflow
/// (that is what `workflow_state` is for); every other field is refused.
pub(super) fn enforce_state_edit(
    ctx: &EngineContext<'_>,
    stored: &HashMap<String, Value>,
    incoming: &HashMap<String, Value>,
) -> Result<()> {
    let Some(workflow) = crate::workflow::global_workflow_engine().get(&ctx.doctype) else {
        return Ok(());
    };
    let state_name = current_state(&workflow, stored);
    let Some(state) = workflow.state(state_name) else {
        // A stored state the workflow no longer declares is not a lock: the
        // transition validation above already rejects moving out of nowhere.
        return Ok(());
    };
    if state.allow_edit.is_empty() {
        return Ok(());
    }
    let allowed = ctx.user.as_ref().is_some_and(|user| {
        user.has_role(SYSTEM_MANAGER_ROLE)
            || state
                .allow_edit
                .iter()
                .any(|role| role == ALL_USER_ROLE || user.has_role(role))
    });
    if allowed {
        return Ok(());
    }
    for key in incoming.keys() {
        if key == "workflow_state" {
            continue;
        }
        return Err(Error::Validation(ValidationError::Invalid(format!(
            "`{key}` cannot be edited while `{}` is in workflow state `{state_name}`",
            ctx.doctype
        ))));
    }
    Ok(())
}
