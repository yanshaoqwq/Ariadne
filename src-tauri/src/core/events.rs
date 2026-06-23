use serde::{Deserialize, Serialize};

use crate::core::ports::PortValue;
use crate::core::workflow::{NodeId, RunControl, RunId, RunStatus, WorkflowId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSeverity {
    Debug,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: String,
    pub timestamp_ms: u64,
    pub severity: EventSeverity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<WorkflowId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    pub event: CoreEvent,
}

impl EventEnvelope {
    pub fn new(
        event_id: impl Into<String>,
        timestamp_ms: u64,
        severity: EventSeverity,
        event: CoreEvent,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            timestamp_ms,
            severity,
            workflow_id: None,
            run_id: None,
            event,
        }
    }

    pub fn with_run(mut self, workflow_id: WorkflowId, run_id: RunId) -> Self {
        self.workflow_id = Some(workflow_id);
        self.run_id = Some(run_id);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreEvent {
    RunControlChanged {
        control: RunControl,
    },
    RunStatusChanged {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<RunStatus>,
        to: RunStatus,
    },
    NodeStarted {
        node_id: NodeId,
    },
    NodeFinished {
        node_id: NodeId,
        #[serde(default)]
        output_ports: Vec<String>,
    },
    NodeFailed {
        node_id: NodeId,
        error: String,
    },
    PortValueProduced {
        node_id: NodeId,
        port_name: String,
        value: PortValue,
    },
    ApprovalRequested {
        node_id: NodeId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt_id: Option<String>,
    },
    ApprovalCompleted {
        node_id: NodeId,
        approved: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    CostRecorded {
        amount_usd: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_id: Option<String>,
    },
    ArtifactCreated {
        artifact_id: String,
    },
    PermissionDenied {
        action: String,
        reason: String,
    },
    ResourceWarning {
        resource: String,
        message: String,
    },
}
