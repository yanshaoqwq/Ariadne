use serde_json::json;

use ariadne::core::{
    ApprovalPolicy, AutoModeState, ExecutionPolicy, LoopPolicy, NodeDefinition, NodeRegistry,
    PermissionPolicy, PermissionRequest, PortDefinition, PortValue, TextRange,
};

#[test]
fn reference_ports_can_be_registered_on_node_definitions() {
    let range = TextRange::new(0, 42).unwrap();
    let value = PortValue::document_ref("doc-1", Some(range));
    assert!(value.is_reference());

    let mut node = NodeDefinition::new("document.patch");
    node.input_ports = vec![PortDefinition::new("source", "document_ref", true)];
    node.output_ports = vec![PortDefinition::new("patch", "artifact_ref", true)];
    node.supports_checkpoint = true;

    let mut registry = NodeRegistry::default();
    registry.register(node).unwrap();

    assert!(registry.contains("document.patch"));
}

#[test]
fn bounded_loop_policy_validates_required_guards() {
    let policy = LoopPolicy {
        max_iterations: 3,
        timeout_ms: 30_000,
        budget_limit_usd: Some(0.5),
        stop_condition: json!({ "kind": "approval_or_score", "score": 0.9 }),
    };

    assert!(policy.validate().is_ok());
}

#[test]
fn auto_mode_skips_confirmation_but_not_hard_permissions() {
    let execution = ExecutionPolicy {
        auto_mode: AutoModeState {
            enabled: true,
            preauthorized_budget_usd: Some(2.0),
        },
        permissions: PermissionPolicy::default(),
    };
    let approval = ApprovalPolicy {
        allow_auto_approval: true,
        approval_prompt_id: Some("default-review".to_owned()),
        require_human_on_conflict: true,
    };

    assert!(execution.should_skip_human_confirmation(&approval, false));
    assert!(execution
        .ensure_permission(&PermissionRequest::WebSearch)
        .is_err());
}
