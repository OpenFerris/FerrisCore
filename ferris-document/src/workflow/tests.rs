use super::*;

fn sample() -> Workflow {
    Workflow {
        name: "Order Approval".to_string(),
        doctype: "Sales Order".to_string(),
        initial_state: "Draft".to_string(),
        states: vec![
            WorkflowState {
                state: "Draft".into(),
                label: None,
                style: StateStyle::Default,
                doc_status: DocStatus::Draft,
                allow_edit: vec!["Sales User".into()],
                allowed_actions: vec![],
                auto_assign: None,
            },
            WorkflowState {
                state: "Approved".into(),
                label: None,
                style: StateStyle::Success,
                doc_status: DocStatus::Submitted,
                allow_edit: vec![],
                allowed_actions: vec![],
                auto_assign: None,
            },
            WorkflowState {
                state: "Rejected".into(),
                label: None,
                style: StateStyle::Danger,
                doc_status: DocStatus::Cancelled,
                allow_edit: vec![],
                allowed_actions: vec![],
                auto_assign: None,
            },
        ],
        transitions: vec![
            WorkflowTransition {
                name: None,
                action: "Approve".into(),
                from_state: "Draft".into(),
                to_state: "Approved".into(),
                allowed_roles: vec!["Sales Manager".into()],
                condition: None,
                required_fields: vec![],
                notify: false,
            },
            WorkflowTransition {
                name: None,
                action: "Reject".into(),
                from_state: "Draft".into(),
                to_state: "Rejected".into(),
                allowed_roles: vec!["Sales Manager".into()],
                condition: Some(WorkflowCondition {
                    field: "grand_total".into(),
                    equals: Value::from(0),
                }),
                required_fields: vec![],
                notify: false,
            },
        ],
    }
}

#[test]
fn test_role_gated_transition() {
    let wf = sample();
    let ctx = HashMap::new();
    // A Sales User cannot approve.
    assert!(
        wf.apply("Draft", "Approve", &["Sales User".to_string()], &ctx)
            .is_err()
    );
    // A Sales Manager can.
    let outcome = wf
        .apply("Draft", "Approve", &["Sales Manager".to_string()], &ctx)
        .unwrap();
    assert_eq!(outcome.state, "Approved");
    assert_eq!(outcome.doc_status, DocStatus::Submitted);
}

#[test]
fn test_condition_blocks_transition() {
    let wf = sample();
    let mut ctx = HashMap::new();
    ctx.insert("grand_total".to_string(), Value::from(500));
    let err = wf
        .apply("Draft", "Reject", &["Sales Manager".to_string()], &ctx)
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Workflow(ferris_core::WorkflowError::ConditionNotMet(_))
    ));

    ctx.insert("grand_total".to_string(), Value::from(0));
    let outcome = wf
        .apply("Draft", "Reject", &["Sales Manager".to_string()], &ctx)
        .unwrap();
    assert_eq!(outcome.state, "Rejected");
}

#[test]
fn test_required_fields_and_transition_event() {
    let mut wf = sample();
    wf.transitions[0].required_fields = vec!["approver".to_string()];
    wf.transitions[0].notify = true;

    let engine = WorkflowEngine::new();
    engine.register(wf).unwrap();

    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<WorkflowEvent>::new()));
    let capture = seen.clone();
    engine.on_transition(std::sync::Arc::new(move |event: &WorkflowEvent| {
        capture.lock().unwrap().push(event.clone());
    }));

    // Missing required field: rejected and no event emitted.
    assert!(
        engine
            .transition(
                "Sales Order",
                "Draft",
                "Approve",
                &["Sales Manager".to_string()],
                &HashMap::new()
            )
            .is_err()
    );
    assert!(seen.lock().unwrap().is_empty());

    let mut ctx = HashMap::new();
    ctx.insert("approver".to_string(), Value::from("manager@example.com"));
    let outcome = engine
        .transition(
            "Sales Order",
            "Draft",
            "Approve",
            &["Sales Manager".to_string()],
            &ctx,
        )
        .unwrap();
    assert_eq!(outcome.state, "Approved");

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].notify);
    assert_eq!(seen[0].doctype, "Sales Order");
    assert_eq!(seen[0].to_state, "Approved");

    assert!(engine.can_transition(
        "Sales Order",
        "Draft",
        "Approve",
        &["Sales Manager".to_string()],
        &ctx
    ));
    assert!(!engine.can_transition(
        "Sales Order",
        "Draft",
        "Approve",
        &["Sales User".to_string()],
        &ctx
    ));
}

#[test]
fn test_engine_registry() {
    let engine = WorkflowEngine::new();
    engine.register(sample()).unwrap();
    // Duplicate doctype is rejected.
    assert!(engine.register(sample()).is_err());
    assert!(engine.get("Sales Order").is_some());

    let outcome = engine
        .apply(
            "Sales Order",
            "Draft",
            "Approve",
            &["Sales Manager".to_string()],
            &HashMap::new(),
        )
        .unwrap();
    assert_eq!(outcome.state, "Approved");
}

/// DOC15: `register` rejects a duplicate doctype **without replacing the
/// incumbent**. `insert`-then-check would have swapped the stored workflow before
/// returning `AlreadyExists`, so a second boot-time registration could silently
/// shadow the first. Pin the first-wins rule.
#[test]
fn a_rejected_registration_keeps_the_first_workflow() {
    let engine = WorkflowEngine::new();
    let first = sample();
    engine.register(first).unwrap();

    // Same doctype, different name and a different initial state.
    let mut clash = sample();
    clash.name = "Shadow".to_string();
    clash.initial_state = "Approved".to_string();
    assert!(engine.register(clash).is_err());

    let stored = engine.get("Sales Order").expect("the first workflow stays");
    assert_eq!(stored.name, "Order Approval");
    assert_eq!(stored.initial_state, "Draft");
}
