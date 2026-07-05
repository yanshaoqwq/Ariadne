use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::contracts::errors::{CoreError, CoreResult};

/// 运行时共享资源类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Llm,
    Embedding,
    Reranker,
    Search,
    Indexing,
    Git,
    Wasm,
    Http,
}

/// 单个操作可使用的资源上限。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ResourceLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_runtime_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
}

/// 单个操作已使用资源。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ResourceUsage {
    pub runtime_ms: u64,
    pub memory_bytes: u64,
    pub output_bytes: u64,
    pub cost_usd: f64,
}

impl ResourceLimits {
    /// 检查资源使用量是否超过任一上限。
    pub fn check_usage(&self, usage: &ResourceUsage) -> CoreResult<()> {
        if let Some(limit) = self.max_runtime_ms {
            if usage.runtime_ms > limit {
                return Err(CoreError::ResourceLimitExceeded {
                    resource: "runtime_ms".to_owned(),
                    reason: format!("usage {} exceeds limit {limit}", usage.runtime_ms),
                });
            }
        }

        if let Some(limit) = self.max_memory_bytes {
            if usage.memory_bytes > limit {
                return Err(CoreError::ResourceLimitExceeded {
                    resource: "memory_bytes".to_owned(),
                    reason: format!("usage {} exceeds limit {limit}", usage.memory_bytes),
                });
            }
        }

        if let Some(limit) = self.max_output_bytes {
            if usage.output_bytes > limit {
                return Err(CoreError::ResourceLimitExceeded {
                    resource: "output_bytes".to_owned(),
                    reason: format!("usage {} exceeds limit {limit}", usage.output_bytes),
                });
            }
        }

        if let Some(limit) = self.max_cost_usd {
            if usage.cost_usd > limit {
                return Err(CoreError::BudgetExceeded {
                    limit_usd: limit,
                    requested_usd: usage.cost_usd,
                });
            }
        }

        Ok(())
    }
}

/// 已获取的资源许可。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourcePermit {
    pub kind: ResourceKind,
    pub units: u32,
}

/// 简单资源池，按 ResourceKind 统计并发占用。
#[derive(Debug, Clone, Default)]
pub struct ResourcePool {
    limits: BTreeMap<ResourceKind, u32>,
    in_use: BTreeMap<ResourceKind, u32>,
}

impl ResourcePool {
    /// 设置资源并发上限。
    pub fn with_limit(mut self, kind: ResourceKind, units: u32) -> Self {
        self.limits.insert(kind, units);
        self
    }

    /// 返回资源当前占用数量。
    pub fn in_use(&self, kind: ResourceKind) -> u32 {
        self.in_use.get(&kind).copied().unwrap_or_default()
    }

    /// 返回资源剩余可用数量；未设置上限时返回 None。
    pub fn available(&self, kind: ResourceKind) -> Option<u32> {
        self.limits
            .get(&kind)
            .map(|limit| limit.saturating_sub(self.in_use(kind)))
    }

    /// 获取资源许可。
    pub fn acquire(&mut self, kind: ResourceKind, units: u32) -> CoreResult<ResourcePermit> {
        if units == 0 {
            return Err(CoreError::validation(
                "resource units must be greater than zero",
            ));
        }

        let Some(limit) = self.limits.get(&kind).copied() else {
            return Ok(ResourcePermit { kind, units });
        };

        let next = self.in_use(kind).saturating_add(units);
        if next > limit {
            return Err(CoreError::ResourceLimitExceeded {
                resource: format!("{kind:?}"),
                reason: format!("requested {next} units exceeds limit {limit}"),
            });
        }

        self.in_use.insert(kind, next);
        Ok(ResourcePermit { kind, units })
    }

    /// 释放资源许可。
    pub fn release(&mut self, permit: ResourcePermit) {
        let current = self.in_use(permit.kind);
        let next = current.saturating_sub(permit.units);
        if next == 0 {
            self.in_use.remove(&permit.kind);
        } else {
            self.in_use.insert(permit.kind, next);
        }
    }
}

/// 可克隆的取消令牌。
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// 创建未取消的令牌。
    pub fn new() -> Self {
        Self::default()
    }

    /// 标记取消。
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// 返回是否已取消。
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// 若已取消则返回 CoreError::Cancelled。
    pub fn check(&self) -> CoreResult<()> {
        if self.is_cancelled() {
            Err(CoreError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_pool_enforces_capacity() {
        let mut pool = ResourcePool::default().with_limit(ResourceKind::Llm, 1);
        let permit = pool.acquire(ResourceKind::Llm, 1).unwrap();

        assert!(pool.acquire(ResourceKind::Llm, 1).is_err());

        pool.release(permit);
        assert!(pool.acquire(ResourceKind::Llm, 1).is_ok());
    }

    #[test]
    fn cancellation_token_reports_cancelled_state() {
        let token = CancellationToken::new();
        assert!(token.check().is_ok());

        token.cancel();
        assert!(matches!(token.check(), Err(CoreError::Cancelled)));
    }
}
