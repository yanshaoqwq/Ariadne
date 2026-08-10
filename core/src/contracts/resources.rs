use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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
