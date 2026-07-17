use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::contracts::errors::{CoreError, CoreResult};

/// 调用方取消后仍可能等待系统调用/阻塞 HTTP timeout 的后台任务全进程上限。
pub(crate) const MAX_DETACHED_BLOCKING_TASKS: usize = 16;
static DETACHED_BLOCKING_TASKS: DetachedBlockingTaskLimiter =
    DetachedBlockingTaskLimiter::new(MAX_DETACHED_BLOCKING_TASKS);

#[derive(Debug)]
struct DetachedBlockingTaskLimiter {
    active: AtomicUsize,
    limit: usize,
}

impl DetachedBlockingTaskLimiter {
    const fn new(limit: usize) -> Self {
        Self {
            active: AtomicUsize::new(0),
            limit,
        }
    }

    fn acquire(&self) -> CoreResult<DetachedBlockingTaskPermit<'_>> {
        let mut current = self.active.load(Ordering::Acquire);
        loop {
            if current >= self.limit {
                return Err(CoreError::ResourceLimitExceeded {
                    resource: "detached_blocking_tasks".to_owned(),
                    reason: format!("active blocking tasks reached process limit {}", self.limit),
                });
            }
            match self.active.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(DetachedBlockingTaskPermit { limiter: self }),
                Err(actual) => current = actual,
            }
        }
    }
}

/// 后台阻塞任务许可；必须由实际 worker 持有到线程退出，不能随调用方取消提前释放。
#[derive(Debug)]
pub(crate) struct DetachedBlockingTaskPermit<'a> {
    limiter: &'a DetachedBlockingTaskLimiter,
}

impl Drop for DetachedBlockingTaskPermit<'_> {
    fn drop(&mut self) {
        self.limiter.active.fetch_sub(1, Ordering::AcqRel);
    }
}

/// 尝试获取一个可脱离调用方生命周期的阻塞任务许可。
pub(crate) fn acquire_detached_blocking_task_permit(
) -> CoreResult<DetachedBlockingTaskPermit<'static>> {
    DETACHED_BLOCKING_TASKS.acquire()
}

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

impl PartialEq for CancellationToken {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.cancelled, &other.cancelled)
            || self.is_cancelled() == other.is_cancelled()
    }
}

impl Eq for CancellationToken {}

/// 外部副作用派发授权。运行时存储提供持久化校验器，执行器在真实副作用边界
/// 调用 `authorize_dispatch`，把运行控制、worker fencing 与 operation journal
/// 收敛到同一个线性化点。独立调用默认使用空授权器。
#[derive(Clone, Default)]
pub struct ExternalDispatchAuthorization {
    inner: Option<Arc<ExternalDispatchAuthorizationInner>>,
}

struct ExternalDispatchAuthorizationInner {
    check: Arc<dyn Fn(bool) -> CoreResult<()> + Send + Sync>,
    sealed: Mutex<bool>,
}

impl ExternalDispatchAuthorization {
    pub fn new(check: impl Fn(bool) -> CoreResult<()> + Send + Sync + 'static) -> Self {
        Self {
            inner: Some(Arc::new(ExternalDispatchAuthorizationInner {
                check: Arc::new(check),
                sealed: Mutex::new(false),
            })),
        }
    }

    /// 只复核运行控制与 fencing，不把 operation 标记为 dispatched。
    pub fn check(&self) -> CoreResult<()> {
        self.invoke(false)
    }

    /// 在真实副作用边界复核并原子登记 dispatched。
    pub fn authorize_dispatch(&self) -> CoreResult<()> {
        self.invoke(true)
    }

    /// 封闭本次执行器持有的授权句柄。所有 clone 共享同一把锁，因此返回后才到达的
    /// 异步派发不能越过 operation 完成事务重新消费旧授权。
    pub fn seal(&self) -> CoreResult<()> {
        let Some(inner) = &self.inner else {
            return Ok(());
        };
        let mut sealed = inner
            .sealed
            .lock()
            .map_err(|_| CoreError::validation("external dispatch authorization lock poisoned"))?;
        *sealed = true;
        Ok(())
    }

    fn invoke(&self, dispatch: bool) -> CoreResult<()> {
        let Some(inner) = &self.inner else {
            return Ok(());
        };
        let sealed = inner
            .sealed
            .lock()
            .map_err(|_| CoreError::validation("external dispatch authorization lock poisoned"))?;
        if *sealed {
            return Err(CoreError::external_cancelled(
                "dispatch_authorization",
                crate::contracts::ExternalDispatchOutcome::NotDispatched,
            ));
        }
        (inner.check)(dispatch)
    }
}

impl fmt::Debug for ExternalDispatchAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExternalDispatchAuthorization")
            .field("enabled", &self.inner.is_some())
            .finish()
    }
}

impl PartialEq for ExternalDispatchAuthorization {
    fn eq(&self, other: &Self) -> bool {
        match (&self.inner, &other.inner) {
            (None, None) => true,
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }
}

impl Eq for ExternalDispatchAuthorization {}

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

    #[test]
    fn c9_detached_blocking_task_limit_is_enforced_and_released() {
        let limiter = DetachedBlockingTaskLimiter::new(MAX_DETACHED_BLOCKING_TASKS);
        let permits = (0..MAX_DETACHED_BLOCKING_TASKS)
            .map(|_| limiter.acquire().unwrap())
            .collect::<Vec<_>>();

        assert!(matches!(
            limiter.acquire(),
            Err(CoreError::ResourceLimitExceeded { ref resource, .. })
                if resource == "detached_blocking_tasks"
        ));

        drop(permits);
        assert!(limiter.acquire().is_ok());
    }
}
