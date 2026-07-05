use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::contracts::{CoreError, CoreResult};
use crate::retrieval::models::{
    FullTextRecord, RebuildReport, RebuildStatus, StoreHealth, StoreStatus, VectorRecord,
};
use crate::retrieval::traits::{FullTextStore, VectorStore};

/// Qdrant sidecar 的启动配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QdrantSidecarConfig {
    pub binary_path: PathBuf,
    pub host: String,
    pub requested_port: u16,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    pub startup_timeout_ms: u64,
}

impl QdrantSidecarConfig {
    /// 根据 host 和实际端口生成 HTTP endpoint。
    pub fn endpoint(&self, port: u16) -> String {
        format!("http://{}:{port}", self.host)
    }
}

/// Qdrant sidecar 当前状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QdrantSidecarStatus {
    pub state: SidecarState,
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl QdrantSidecarStatus {
    /// 构造停止状态。
    fn stopped(host: impl Into<String>) -> Self {
        Self {
            state: SidecarState::Stopped,
            host: host.into(),
            port: None,
            endpoint: None,
            process_id: None,
            reason: None,
        }
    }
}

/// sidecar 生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidecarState {
    Stopped,
    Running,
    Degraded,
    Unavailable,
}

/// 端口选择结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortSelection {
    pub port: u16,
    pub reused_requested_port: bool,
}

/// 后端自动恢复动作。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalRecoveryAction {
    RestartSidecar,
    RebuildVectorIndex,
    RebuildFullTextIndex,
}

/// 后端自动恢复报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalRecoveryReport {
    #[serde(default)]
    pub actions: Vec<RetrievalRecoveryAction>,
    #[serde(default)]
    pub sidecar_status: Option<QdrantSidecarStatus>,
    #[serde(default)]
    pub rebuild_reports: Vec<RebuildReport>,
}

/// 内部端口预留结果；listener 保持到 spawn 前一刻，缩小端口被抢占窗口。
struct ReservedPortSelection {
    selection: PortSelection,
    listener: TcpListener,
}

/// sidecar 进程启动器，测试可替换该接口避免真的启动 Qdrant。
pub trait SidecarProcessRunner: Send + Sync {
    /// 启动 sidecar 进程。
    fn spawn(&self, config: &QdrantSidecarConfig, port: u16) -> CoreResult<Child>;
}

/// 基于 std::process::Command 的默认进程启动器。
#[derive(Debug, Default)]
pub struct CommandSidecarProcessRunner;

impl SidecarProcessRunner for CommandSidecarProcessRunner {
    /// 创建数据/日志目录，并通过环境变量传递 Qdrant 基础配置。
    fn spawn(&self, config: &QdrantSidecarConfig, port: u16) -> CoreResult<Child> {
        std::fs::create_dir_all(&config.data_dir)?;
        std::fs::create_dir_all(&config.log_dir)?;

        Command::new(&config.binary_path)
            .env("QDRANT__SERVICE__HOST", &config.host)
            .env("QDRANT__SERVICE__HTTP_PORT", port.to_string())
            .env("QDRANT__STORAGE__STORAGE_PATH", &config.data_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(CoreError::from)
    }
}

/// 管理 Qdrant sidecar 的生命周期和健康状态。
pub struct QdrantSidecarSupervisor<R = CommandSidecarProcessRunner> {
    config: QdrantSidecarConfig,
    runner: R,
    child: Mutex<Option<Child>>,
    status: Mutex<QdrantSidecarStatus>,
}

impl QdrantSidecarSupervisor {
    /// 创建使用默认命令启动器的 supervisor。
    pub fn new(config: QdrantSidecarConfig) -> Self {
        Self::with_runner(config, CommandSidecarProcessRunner)
    }
}

impl<R> QdrantSidecarSupervisor<R>
where
    R: SidecarProcessRunner,
{
    /// 创建可注入进程启动器的 supervisor。
    pub fn with_runner(config: QdrantSidecarConfig, runner: R) -> Self {
        let status = QdrantSidecarStatus::stopped(config.host.clone());
        Self {
            config,
            runner,
            child: Mutex::new(None),
            status: Mutex::new(status),
        }
    }

    /// 启动 sidecar，并在端口冲突或健康检查失败时标记 degraded/unavailable。
    pub fn start(&self) -> CoreResult<QdrantSidecarStatus> {
        if let Some(existing) = self.status_if_running()? {
            return Ok(existing);
        }

        let reservation = reserve_available_port(&self.config.host, self.config.requested_port)?;
        let selection = reservation.selection.clone();
        // 外部 sidecar 需要自己 bind 端口；这里只能在 spawn 前释放预留 listener。
        drop(reservation.listener);
        let child = match self.runner.spawn(&self.config, selection.port) {
            Ok(child) => child,
            Err(error) => {
                // 进程完全无法启动时记录不可用状态，便于前端诊断。
                let status = QdrantSidecarStatus {
                    state: SidecarState::Unavailable,
                    host: self.config.host.clone(),
                    port: Some(selection.port),
                    endpoint: Some(self.config.endpoint(selection.port)),
                    process_id: None,
                    reason: Some(error.to_string()),
                };
                *self.status.lock().map_err(lock_error)? = status;
                return Err(error);
            }
        };
        let process_id = child.id();
        *self.child.lock().map_err(lock_error)? = Some(child);

        // 端口冲突时仍可继续运行，但需要向诊断层暴露 degraded 原因。
        let mut state = if selection.reused_requested_port {
            SidecarState::Running
        } else {
            SidecarState::Degraded
        };
        let mut reason = if selection.reused_requested_port {
            None
        } else {
            Some(format!(
                "requested port {} was unavailable; selected {}",
                self.config.requested_port, selection.port
            ))
        };

        if let Err(error) = wait_for_tcp_health(
            &self.config.host,
            selection.port,
            self.config.startup_timeout_ms,
        ) {
            // 进程已启动但 TCP 不可达，保留进程信息并报告 degraded。
            state = SidecarState::Degraded;
            reason = Some(error.to_string());
        }

        let status = QdrantSidecarStatus {
            state,
            host: self.config.host.clone(),
            port: Some(selection.port),
            endpoint: Some(self.config.endpoint(selection.port)),
            process_id: Some(process_id),
            reason,
        };
        *self.status.lock().map_err(lock_error)? = status.clone();
        Ok(status)
    }

    /// 停止当前 sidecar 进程。
    pub fn stop(&self) -> CoreResult<QdrantSidecarStatus> {
        if let Some(mut child) = self.child.lock().map_err(lock_error)?.take() {
            child.kill()?;
            let _ = child.wait();
        }

        let status = QdrantSidecarStatus::stopped(self.config.host.clone());
        *self.status.lock().map_err(lock_error)? = status.clone();
        Ok(status)
    }

    /// 标记进程崩溃或被外部终止。
    pub fn mark_crashed(&self, reason: impl Into<String>) -> CoreResult<QdrantSidecarStatus> {
        let mut status = self.status.lock().map_err(lock_error)?;
        status.state = SidecarState::Unavailable;
        status.reason = Some(reason.into());
        Ok(status.clone())
    }

    /// 重启 sidecar。
    pub fn restart(&self) -> CoreResult<QdrantSidecarStatus> {
        self.stop()?;
        self.start()
    }

    /// sidecar 不可用或降级时尝试自动重启。
    pub fn recover_if_unavailable(&self) -> CoreResult<Option<QdrantSidecarStatus>> {
        let status = self.status()?;
        if matches!(
            status.state,
            SidecarState::Unavailable | SidecarState::Degraded | SidecarState::Stopped
        ) {
            return self.restart().map(Some);
        }
        Ok(None)
    }

    /// 返回当前 sidecar 状态快照。
    pub fn status(&self) -> CoreResult<QdrantSidecarStatus> {
        Ok(self.status.lock().map_err(lock_error)?.clone())
    }

    /// 将 sidecar 状态转换成通用 StoreHealth。
    pub fn health_check(&self) -> CoreResult<StoreHealth> {
        let status = self.status()?;
        let reason = status
            .reason
            .unwrap_or_else(|| "sidecar stopped".to_owned());
        match status.state {
            SidecarState::Running => Ok(StoreHealth::healthy("qdrant_sidecar")),
            SidecarState::Degraded => Ok(StoreHealth::degraded("qdrant_sidecar", reason)),
            SidecarState::Stopped => Ok(StoreHealth::unavailable("qdrant_sidecar", reason)),
            SidecarState::Unavailable => Ok(StoreHealth {
                component: "qdrant_sidecar".to_owned(),
                status: StoreStatus::Unavailable,
                reason: Some(reason),
            }),
        }
    }

    /// running/degraded 都表示已有进程状态，不重复启动。
    fn status_if_running(&self) -> CoreResult<Option<QdrantSidecarStatus>> {
        let status = self.status()?;
        Ok(
            matches!(status.state, SidecarState::Running | SidecarState::Degraded)
                .then_some(status),
        )
    }
}

/// 选择可用端口；请求端口不可用时回退到系统分配端口。
pub fn select_available_port(host: &str, requested_port: u16) -> CoreResult<PortSelection> {
    reserve_available_port(host, requested_port).map(|reservation| reservation.selection)
}

/// 选择并暂时持有可用端口，供启动流程在 spawn 前一刻释放。
fn reserve_available_port(host: &str, requested_port: u16) -> CoreResult<ReservedPortSelection> {
    if requested_port == 0 {
        return reserve_ephemeral_port(host).map(|(port, listener)| ReservedPortSelection {
            selection: PortSelection {
                port,
                reused_requested_port: false,
            },
            listener,
        });
    }

    if let Ok(listener) = TcpListener::bind((host, requested_port)) {
        return Ok(ReservedPortSelection {
            selection: PortSelection {
                port: requested_port,
                reused_requested_port: true,
            },
            listener,
        });
    }

    reserve_ephemeral_port(host).map(|(port, listener)| ReservedPortSelection {
        selection: PortSelection {
            port,
            reused_requested_port: false,
        },
        listener,
    })
}

/// 检查 host:port 当前是否可绑定。
pub fn is_port_available(host: &str, port: u16) -> bool {
    TcpListener::bind((host, port)).is_ok()
}

/// 等待 TCP 端口可连接，用于启动后的轻量健康检查。
pub fn wait_for_tcp_health(host: &str, port: u16, timeout_ms: u64) -> CoreResult<()> {
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if TcpStream::connect((host, port)).is_ok() {
            return Ok(());
        }

        if std::time::Instant::now() >= deadline {
            return Err(CoreError::External {
                service: "qdrant_sidecar".to_owned(),
                message: format!("timed out waiting for {host}:{port}"),
            });
        }

        std::thread::sleep(Duration::from_millis(25));
    }
}

/// 向系统申请一个临时可用端口。
fn reserve_ephemeral_port(host: &str) -> CoreResult<(u16, TcpListener)> {
    let listener = TcpListener::bind((host, 0))?;
    let port = listener.local_addr()?.port();
    Ok((port, listener))
}

/// 将锁中毒转换成统一错误。
fn lock_error<T>(error: std::sync::PoisonError<T>) -> CoreError {
    CoreError::validation(format!("sidecar supervisor lock poisoned: {error}"))
}

/// 汇总 sidecar 重启和索引重建自动恢复动作。
pub fn recover_retrieval_components<R>(
    sidecar: &QdrantSidecarSupervisor<R>,
    vector_store: &dyn VectorStore,
    vector_records: Vec<VectorRecord>,
    full_text_store: &dyn FullTextStore,
    full_text_records: Vec<FullTextRecord>,
) -> CoreResult<RetrievalRecoveryReport>
where
    R: SidecarProcessRunner,
{
    let mut report = RetrievalRecoveryReport {
        actions: Vec::new(),
        sidecar_status: None,
        rebuild_reports: Vec::new(),
    };

    if let Some(status) = sidecar.recover_if_unavailable()? {
        report.actions.push(RetrievalRecoveryAction::RestartSidecar);
        report.sidecar_status = Some(status);
    }

    if vector_store.health_check()?.status == StoreStatus::RebuildRequired {
        let rebuild = vector_store.rebuild_from_records(vector_records)?;
        if rebuild.status == RebuildStatus::Completed {
            report
                .actions
                .push(RetrievalRecoveryAction::RebuildVectorIndex);
        }
        report.rebuild_reports.push(rebuild);
    }

    if full_text_store.health_check()?.status == StoreStatus::RebuildRequired {
        let rebuild = full_text_store.rebuild_from_records(full_text_records)?;
        if rebuild.status == RebuildStatus::Completed {
            report
                .actions
                .push(RetrievalRecoveryAction::RebuildFullTextIndex);
        }
        report.rebuild_reports.push(rebuild);
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_selection_falls_back_when_requested_port_is_taken() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let requested = listener.local_addr().unwrap().port();

        let selection = select_available_port("127.0.0.1", requested).unwrap();

        assert_ne!(selection.port, requested);
        assert!(!selection.reused_requested_port);
    }
}
