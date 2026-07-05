use ariadne::contracts::NodeId;
use ariadne::diagnostics::{BackendDiagnosticsReport, DiagnosticStatus};
use ariadne::providers::{ProviderHealth, ProviderHealthReport, ProviderKind};
use ariadne::retrieval::{StoreHealth, StoreStatus};
use ariadne::workflow::{
    RuntimeRecoveryReport, RuntimeReferenceCheck, RuntimeReferenceKind, RuntimeStoreHealth,
};

#[test]
fn backend_diagnostics_aggregates_provider_sidecar_and_runtime_failures() {
    let mut recovery = RuntimeRecoveryReport::new();
    recovery.checked_reference_count = 1;
    recovery.missing_references.push(RuntimeReferenceCheck {
        kind: RuntimeReferenceKind::Artifact,
        id: "artifact-1".to_owned(),
        node_id: NodeId::from("writer"),
        field_name: "outputs.artifact".to_owned(),
        exists: false,
        message: "artifact missing".to_owned(),
    });

    let report = BackendDiagnosticsReport::collect(
        RuntimeStoreHealth::Missing,
        Some(recovery),
        vec![
            ProviderHealthReport {
                provider_id: "openai".to_owned(),
                kind: ProviderKind::Llm,
                health: ProviderHealth::Healthy,
            },
            ProviderHealthReport {
                provider_id: "search".to_owned(),
                kind: ProviderKind::Search,
                health: ProviderHealth::Unhealthy {
                    reason: "connection refused".to_owned(),
                },
            },
        ],
        vec![StoreHealth {
            component: "qdrant_sidecar".to_owned(),
            status: StoreStatus::Unavailable,
            reason: Some("process exited".to_owned()),
        }],
    );

    assert_eq!(report.status, DiagnosticStatus::Unavailable);
    assert!(report.requires_attention());
    assert!(report
        .items
        .iter()
        .any(|item| item.component == "provider.search.search"
            && item.status == DiagnosticStatus::Unavailable));
    assert!(report
        .items
        .iter()
        .any(|item| item.component == "qdrant_sidecar"
            && item.status == DiagnosticStatus::Unavailable));
    assert!(report.items.iter().any(|item| {
        item.component == "workflow_runtime_recovery"
            && item.status == DiagnosticStatus::Degraded
            && item
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("runtime references are missing"))
    }));
}

#[test]
fn backend_diagnostics_reports_clean_when_all_components_are_healthy() {
    let report = BackendDiagnosticsReport::collect(
        RuntimeStoreHealth::Healthy,
        Some(RuntimeRecoveryReport::new()),
        vec![ProviderHealthReport {
            provider_id: "fast".to_owned(),
            kind: ProviderKind::Llm,
            health: ProviderHealth::Healthy,
        }],
        vec![StoreHealth::healthy("memory_full_text_store")],
    );

    assert_eq!(report.status, DiagnosticStatus::Healthy);
    assert!(!report.requires_attention());
}
