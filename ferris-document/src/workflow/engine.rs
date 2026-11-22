use super::*;

/// Registry of workflows keyed by doctype.
pub struct WorkflowEngine {
    workflows: RwLock<HashMap<String, Workflow>>,
    listeners: RwLock<Vec<std::sync::Arc<WorkflowListener>>>,
}

impl WorkflowEngine {
    pub fn new() -> Self {
        Self {
            workflows: RwLock::new(HashMap::new()),
            listeners: RwLock::new(Vec::new()),
        }
    }

    /// Register a listener invoked after every successful transition.
    pub fn on_transition(&self, listener: std::sync::Arc<WorkflowListener>) {
        self.listeners.write().push(listener);
    }

    /// Whether `roles` may trigger `action` from `from_state`.
    pub fn can_transition(
        &self,
        doctype: &str,
        from_state: &str,
        action: &str,
        roles: &[String],
        context: &HashMap<String, Value>,
    ) -> bool {
        self.get(doctype).is_some_and(|workflow| {
            workflow
                .find_transition(from_state, action, roles, context)
                .is_some()
        })
    }

    /// Apply a transition and emit a [`WorkflowEvent`] to registered listeners.
    pub fn transition(
        &self,
        doctype: &str,
        from_state: &str,
        action: &str,
        roles: &[String],
        context: &HashMap<String, Value>,
    ) -> Result<TransitionOutcome> {
        let workflow = self.get(doctype).ok_or_else(|| {
            Error::Workflow(ferris_core::WorkflowError::NotFound(doctype.to_string()))
        })?;
        let outcome = workflow.apply(from_state, action, roles, context)?;

        let notify = workflow
            .find_transition(from_state, action, roles, context)
            .map(|t| t.notify)
            .unwrap_or(false);
        let event = WorkflowEvent {
            // DOC15: `workflow` is owned here, so move the name instead of
            // cloning it; `workflow` is not used again after this.
            workflow: workflow.name,
            doctype: doctype.to_string(),
            action: action.to_string(),
            from_state: from_state.to_string(),
            to_state: outcome.state.clone(),
            notify,
            timestamp: chrono::Utc::now(),
        };
        self.dispatch(&event);
        Ok(outcome)
    }

    /// Emit the event for a transition the document engine already validated
    /// and persisted.
    ///
    /// [`Self::transition`] validates *and* notifies in one call, which is the
    /// right shape for a caller that owns the whole move. An update validates
    /// `workflow_state` before the write and must not announce the change until
    /// it commits, so the notification half is reachable on its own (DOC-09).
    /// A doctype whose workflow was never registered, or an edge the workflow
    /// no longer declares, emits nothing rather than inventing an event.
    pub fn emit(&self, doctype: &str, from_state: &str, outcome: &TransitionOutcome) {
        let Some(workflow) = self.get(doctype) else {
            return;
        };
        let Some(transition) = workflow
            .transitions
            .iter()
            .find(|t| t.from_state == from_state && t.to_state == outcome.state)
        else {
            return;
        };
        let event = WorkflowEvent {
            workflow: workflow.name.clone(),
            doctype: doctype.to_string(),
            action: transition.action.clone(),
            from_state: from_state.to_string(),
            to_state: outcome.state.clone(),
            notify: transition.notify,
            timestamp: chrono::Utc::now(),
        };
        self.dispatch(&event);
    }

    /// Hand an event to every registered listener.
    fn dispatch(&self, event: &WorkflowEvent) {
        let listeners = self.listeners.read().clone();
        for listener in listeners {
            listener(event);
        }
    }

    pub fn register(&self, workflow: Workflow) -> Result<()> {
        // DOC15: the check and the insert happen under **one** write guard, via
        // `entry`. A read-guard `contains_key` followed by a write-guard
        // `insert` would let two concurrent registrations of the same doctype
        // both pass the check; and `insert`-then-check would replace the
        // incumbent (first registration must win).
        let doctype = workflow.doctype.clone();
        let mut workflows = self.workflows.write();
        match workflows.entry(doctype.clone()) {
            std::collections::hash_map::Entry::Occupied(_) => {
                return Err(Error::Workflow(ferris_core::WorkflowError::AlreadyExists(
                    doctype,
                )));
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(workflow);
            }
        }
        drop(workflows);
        Ok(())
    }

    pub fn get(&self, doctype: &str) -> Option<Workflow> {
        self.workflows.read().get(doctype).cloned()
    }

    pub fn list(&self) -> Vec<Workflow> {
        self.workflows.read().values().cloned().collect()
    }

    /// Apply an action to a doctype's workflow.
    pub fn apply(
        &self,
        doctype: &str,
        from_state: &str,
        action: &str,
        roles: &[String],
        context: &HashMap<String, Value>,
    ) -> Result<TransitionOutcome> {
        let workflow = self.get(doctype).ok_or_else(|| {
            Error::Workflow(ferris_core::WorkflowError::NotFound(doctype.to_string()))
        })?;
        workflow.apply(from_state, action, roles, context)
    }
}

impl Default for WorkflowEngine {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL_ENGINE: LazyLock<WorkflowEngine> = LazyLock::new(WorkflowEngine::new);

/// Process-wide workflow registry.
pub fn global_workflow_engine() -> &'static WorkflowEngine {
    &GLOBAL_ENGINE
}
