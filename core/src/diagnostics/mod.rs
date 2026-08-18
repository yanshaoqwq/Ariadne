use serde::{Deserialize, Serialize};

use crate::providers::{ProviderHealth, ProviderHealthReport, ProviderKind};
use crate::retrieval::{StoreHealth, StoreStatus};
use crate::workflow::{RuntimeRecoveryReport, RuntimeStoreHealth};

/// 统一后端诊断状态，供 IPC/UI 展示和恢复策略判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticStatus {
    Healthy,
    Degraded,
    Unavailable,
}

/// 单个后端组件诊断项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticItem {
    pub component: String,
    pub status: DiagnosticStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// 后端恢复/降级诊断报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendDiagnosticsReport {
    pub status: DiagnosticStatus,
    #[serde(default)]
    pub items: Vec<DiagnosticItem>,
}

impl BackendDiagnosticsReport {
    /// 从各模块健康报告汇总后端诊断状态。
    pub fn collect(
        runtime_store: RuntimeStoreHealth,
        runtime_recovery: Option<RuntimeRecoveryReport>,
        provider_reports: Vec<ProviderHealthReport>,
        retrieval_reports: Vec<StoreHealth>,
    ) -> Self {
        let mut items = Vec::new();
        items.push(runtime_store_item(runtime_store));
        if let Some(report) = runtime_recovery {
            items.push(runtime_recovery_item(report));
        }
        items.extend(provider_reports.into_iter().map(provider_item));
        items.extend(retrieval_reports.into_iter().map(retrieval_item));
        let status = aggregate_status(&items);
        Self { status, items }
    }

    /// U172-A：只含应用级组件的空报告，供**无打开项目**时使用。
    ///
    /// 为什么需要它：`collect` 的四个入参全部以项目根为前提
    /// （runtime.db / 运行恢复 / 项目检索运行时都在项目目录内），
    /// 无项目时一个都取不到。但诊断里有一半组件是应用级的
    /// （凭据保护是应用状态目录里的一个文件，与项目无关），
    /// 而「还没开项目」恰恰是用户最需要查「密钥存哪了、为什么连不上模型」的时刻。
    ///
    /// 空 items + Healthy 是刻意的：此刻确实没有任何**项目级**组件可判，
    /// 调用方随后 `extend_items` 追加应用级项，总状态由 `aggregate_status` 重算。
    /// 若这里预置一条「没有项目」的 Degraded 项，就等于把「没开项目」
    /// 说成一种故障——它不是故障，是正常的启动状态。
    pub fn app_scope_only() -> Self {
        Self {
            status: DiagnosticStatus::Healthy,
            items: Vec::new(),
        }
    }

    /// 返回是否存在降级或不可用组件。
    pub fn requires_attention(&self) -> bool {
        self.status != DiagnosticStatus::Healthy
    }

    /// 追加命令层可直接检查的配置诊断项，并重新聚合总状态。
    pub fn extend_items(&mut self, items: impl IntoIterator<Item = DiagnosticItem>) {
        self.items.extend(items);
        self.status = aggregate_status(&self.items);
    }
}

fn runtime_store_item(health: RuntimeStoreHealth) -> DiagnosticItem {
    match health {
        RuntimeStoreHealth::Healthy => DiagnosticItem {
            component: "runtime.db".to_owned(),
            status: DiagnosticStatus::Healthy,
            reason: None,
        },
        RuntimeStoreHealth::Missing => DiagnosticItem {
            component: "runtime.db".to_owned(),
            status: DiagnosticStatus::Degraded,
            reason: Some("runtime.db is missing and will need initialization".to_owned()),
        },
        RuntimeStoreHealth::Corrupt { message } => DiagnosticItem {
            component: "runtime.db".to_owned(),
            status: DiagnosticStatus::Unavailable,
            reason: Some(message),
        },
    }
}

fn runtime_recovery_item(report: RuntimeRecoveryReport) -> DiagnosticItem {
    if report.is_clean() {
        return DiagnosticItem {
            component: "workflow_runtime_recovery".to_owned(),
            status: DiagnosticStatus::Healthy,
            reason: None,
        };
    }
    let mut reasons = Vec::new();
    if !report.missing_references.is_empty() {
        reasons.push(format!(
            "{} runtime references are missing",
            report.missing_references.len()
        ));
    }
    reasons.extend(report.degraded_reasons);
    DiagnosticItem {
        component: "workflow_runtime_recovery".to_owned(),
        status: DiagnosticStatus::Degraded,
        reason: Some(reasons.join("; ")),
    }
}

fn provider_item(report: ProviderHealthReport) -> DiagnosticItem {
    let component = format!(
        "provider.{}.{}",
        provider_kind_label(report.kind),
        report.provider_id
    );
    match report.health {
        ProviderHealth::Healthy => DiagnosticItem {
            component,
            status: DiagnosticStatus::Healthy,
            reason: None,
        },
        ProviderHealth::Degraded { reason } => DiagnosticItem {
            component,
            status: DiagnosticStatus::Degraded,
            reason: Some(reason),
        },
        ProviderHealth::Unhealthy { reason } => DiagnosticItem {
            component,
            status: DiagnosticStatus::Unavailable,
            reason: Some(reason),
        },
    }
}

fn retrieval_item(health: StoreHealth) -> DiagnosticItem {
    let status = match health.status {
        StoreStatus::Healthy => DiagnosticStatus::Healthy,
        StoreStatus::Degraded | StoreStatus::RebuildRequired => DiagnosticStatus::Degraded,
        StoreStatus::Unavailable => DiagnosticStatus::Unavailable,
    };
    DiagnosticItem {
        component: health.component,
        status,
        reason: health.reason,
    }
}

fn aggregate_status(items: &[DiagnosticItem]) -> DiagnosticStatus {
    if items
        .iter()
        .any(|item| item.status == DiagnosticStatus::Unavailable)
    {
        DiagnosticStatus::Unavailable
    } else if items
        .iter()
        .any(|item| item.status == DiagnosticStatus::Degraded)
    {
        DiagnosticStatus::Degraded
    } else {
        DiagnosticStatus::Healthy
    }
}

fn provider_kind_label(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Llm => "llm",
        ProviderKind::Embedding => "embedding",
        ProviderKind::Reranker => "reranker",
        ProviderKind::Search => "search",
    }
}
