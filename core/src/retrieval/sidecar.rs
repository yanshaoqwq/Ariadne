use std::net::{TcpListener, TcpStream, ToSocketAddrs};
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
    /// 冷却窗口内允许的最大自动重启次数。默认 3。
    ///
    /// 节流是**安全需求**而非调优：sidecar 若因磁盘损坏、端口被占、二进制缺失
    /// 这类持久故障起不来，无节流的自动恢复会在每次诊断刷新时拉起一个必然失败的
    /// 进程，把故障放大成 fork 风暴。
    #[serde(default = "default_max_restarts_per_window")]
    pub max_restarts_per_window: u32,
    /// 重启计数的滑动窗口长度（毫秒）。默认 60 秒。
    #[serde(default = "default_restart_window_ms")]
    pub restart_window_ms: u64,
}

/// 默认冷却窗口内最多重启 3 次：够覆盖偶发崩溃，又不至于在持久故障下失控。
/// `pub(crate)` 而非 `pub`：组合根（`retrieval/project.rs`）要用它填字面量，
/// 但它是 serde 默认值的实现细节，不属于对外契约。
pub(crate) fn default_max_restarts_per_window() -> u32 {
    3
}

/// 默认滑动窗口 60 秒。
pub(crate) fn default_restart_window_ms() -> u64 {
    60_000
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
    /// 滑动窗口内最近一次自动重启的时刻。自动重启失败后记下时间点，
    /// 供 `allow_auto_restart` 判断窗口内是否已达上限。
    restart_window: Mutex<Vec<std::time::Instant>>,
    /// 最近一次**恢复用**探活的时刻，供 `recover_if_unavailable` 节流。
    /// 只记恢复路径，不记 `probe()`/`health_check()` 的直接调用——
    /// 诊断路径要的是当下真实状态，不能被检索路径的节流窗口糊住。
    last_recovery_probe: Mutex<Option<std::time::Instant>>,
}

impl<R> Drop for QdrantSidecarSupervisor<R> {
    fn drop(&mut self) {
        if let Ok(child) = self.child.get_mut() {
            if let Some(mut child) = child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
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
            restart_window: Mutex::new(Vec::new()),
            last_recovery_probe: Mutex::new(None),
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

    /// 探活并按结果刷新缓存状态，返回刷新后的状态。
    ///
    /// **为什么必须探活**：`status` 字段此前只有 `start`/`stop`/`mark_crashed` 三个写入点，
    /// 而 `mark_crashed` 在生产中没有调用者。sidecar 被 OOM 或外部 kill 掉后没人改状态，
    /// 缓存会永远停在 `Running`，诊断页显示健康而所有向量检索静默失败——
    /// 这比"没有自动恢复"更糟，因为它会骗人。
    ///
    /// **判据顺序**：先 `try_wait()` 问内核要进程死活（不阻塞、结论权威），
    /// 只有进程确实活着才做一次 TCP 连接。反过来先连 TCP 的话，
    /// 进程已死时要白等一个连接超时才能得出结论。
    ///
    /// **只观测不恢复**：本函数不重启。诊断是只读路径，让"看一眼设置页"
    /// 产生重启后端服务的副作用是危险的；恢复由 `recover_if_unavailable` 显式发起。
    pub fn probe(&self) -> CoreResult<QdrantSidecarStatus> {
        // Stopped 是我们自己 stop() 出来的确定状态，没有进程可探，探了反而会把
        // "用户主动停止" 误报成 "崩溃"。
        {
            let status = self.status.lock().map_err(lock_error)?;
            if status.state == SidecarState::Stopped {
                return Ok(status.clone());
            }
        }

        // 先取 child 再取 status，与 stop()/start() 保持同一顺序，避免锁序反转死锁。
        let exit_reason = {
            let mut child_slot = self.child.lock().map_err(lock_error)?;
            match child_slot.as_mut() {
                // try_wait 返回 Some 表示进程已退出；同时 reap 掉僵尸进程。
                Some(child) => match child.try_wait()? {
                    Some(exit_status) => {
                        *child_slot = None;
                        Some(format!("sidecar process exited: {exit_status}"))
                    }
                    None => None,
                },
                // 状态非 Stopped 却没有 child 句柄：进程不由本 supervisor 持有
                // （外部 Qdrant），只能靠 TCP 判断。
                None => None,
            }
        };

        if let Some(reason) = exit_reason {
            return self.mark_crashed(reason);
        }

        let (host, port) = {
            let status = self.status.lock().map_err(lock_error)?;
            (status.host.clone(), status.port)
        };
        let Some(port) = port else {
            // 没有端口说明从未成功启动过，保持既有状态不动。
            return self.status();
        };

        // 单次带超时连接，绝不能复用 wait_for_tcp_health——那个是 25ms 一轮的阻塞轮询，
        // 放进诊断路径会让 sidecar 真死时整个设置页卡满 startup_timeout_ms。
        if let Err(error) = probe_tcp_once(&host, port, PROBE_CONNECT_TIMEOUT_MS) {
            let mut status = self.status.lock().map_err(lock_error)?;
            // 进程还在但端口不通：降级而非不可用——可能只是负载高或正在恢复，
            // 报成 Unavailable 会诱发不必要的重启。
            status.state = SidecarState::Degraded;
            status.reason = Some(error.to_string());
            return Ok(status.clone());
        }

        let mut status = self.status.lock().map_err(lock_error)?;
        // 端口通了就清掉旧的失败原因，否则一次瞬时抖动的 reason 会永久粘在诊断上。
        // 但端口回退导致的 Degraded 要保留：那个降级原因与连通性无关，探活无权清除。
        if status.state != SidecarState::Degraded || !is_port_fallback_reason(&status.reason) {
            status.state = SidecarState::Running;
            status.reason = None;
        }
        Ok(status.clone())
    }

    /// 重启 sidecar。
    pub fn restart(&self) -> CoreResult<QdrantSidecarStatus> {
        self.stop()?;
        self.start()
    }

    /// sidecar 不可用或降级时尝试自动重启。
    ///
    /// 依赖 `probe()` 刷新过的状态；直接读缓存会在进程被外部杀掉时永远返回 `None`。
    ///
    /// **节流是安全需求**：滑动窗口内达到 `max_restarts_per_window` 上限后拒绝再重启，
    /// 避免持久故障（磁盘损坏、端口被占、二进制缺失）下每次诊断刷新都拉起一个
    /// 必然失败的进程，把一次故障放大成 fork 风暴。
    ///
    /// **不恢复 `Stopped`**：那是用户主动 stop() 出来的状态，自动拉起等于无视用户的
    /// 停止意图（比如用户因检索异常决定停用向量检索）。
    pub fn recover_if_unavailable(&self) -> CoreResult<Option<QdrantSidecarStatus>> {
        if !self.recovery_probe_due()? {
            return Ok(None);
        }
        let status = self.probe()?;
        if matches!(
            status.state,
            SidecarState::Unavailable | SidecarState::Degraded
        ) {
            if !self.allow_auto_restart()? {
                return Ok(None);
            }
            return self.restart().map(Some);
        }
        Ok(None)
    }

    /// 清掉恢复探活的节流窗口，使下一次 `recover_if_unavailable` 必定真的探活。
    ///
    /// 仅供测试：验证「重启上限」时必须逐轮清窗口，否则被截住的是节流而非上限，
    /// 测试会在缺陷仍在的情况下通过。生产路径不该调用——绕过节流就是绕过性能保护。
    pub fn reset_recovery_probe_throttle_for_tests(&self) {
        if let Ok(mut last) = self.last_recovery_probe.lock() {
            *last = None;
        }
    }

    /// 恢复路径本次是否该真的探活。
    ///
    /// 到期即**立刻记下时刻**（而不是等 probe 返回后再记）：probe 失败会走 `?` 提前
    /// 返回，若把记录放在后面，一个持续失败的 sidecar 会让每次检索都重新探活，
    /// 正好在故障时丢掉节流。
    fn recovery_probe_due(&self) -> CoreResult<bool> {
        let mut last = self.last_recovery_probe.lock().map_err(lock_error)?;
        let now = std::time::Instant::now();
        if let Some(at) = *last {
            if now.duration_since(at)
                < std::time::Duration::from_millis(RECOVERY_PROBE_MIN_INTERVAL_MS)
            {
                return Ok(false);
            }
        }
        *last = Some(now);
        Ok(true)
    }

    /// 滑动窗口内是否还允许一次自动重启。
    ///
    /// 窗口内重启次数达上限时返回 false，并**不重置**窗口——重置会让冷却被
    /// 一次成功的探活清零，起不到抑制持续故障的作用。
    fn allow_auto_restart(&self) -> CoreResult<bool> {
        let mut window = self.restart_window.lock().map_err(lock_error)?;
        let window_ms = self.config.restart_window_ms;
        let deadline =
            std::time::Instant::now().checked_sub(std::time::Duration::from_millis(window_ms));
        if let Some(deadline) = deadline {
            window.retain(|at| *at > deadline);
        } else {
            // 理论不可能：restart_window_ms 是 u64，极端值溢出才走到这。保守清空。
            window.clear();
        }
        if window.len() >= self.config.max_restarts_per_window as usize {
            return Ok(false);
        }
        window.push(std::time::Instant::now());
        Ok(true)
    }

    /// 返回当前 sidecar 状态快照。
    pub fn status(&self) -> CoreResult<QdrantSidecarStatus> {
        Ok(self.status.lock().map_err(lock_error)?.clone())
    }

    /// 将 sidecar 状态转换成通用 StoreHealth。
    ///
    /// 先探活再转换：只读缓存字段的话，进程被 OOM 杀掉后这里会一直报 healthy。
    /// 探活开销是一次 `try_wait()` 加一次带超时的 TCP 连接，不阻塞诊断路径。
    pub fn health_check(&self) -> CoreResult<StoreHealth> {
        let status = self.probe()?;
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
    ///
    /// 这里必须探活而非读缓存：进程被外部杀掉后缓存仍是 `Running`，
    /// `start()` 会据此判定"已在运行"直接返回，`restart()` 也就永远拉不起新进程。
    fn status_if_running(&self) -> CoreResult<Option<QdrantSidecarStatus>> {
        let status = self.probe()?;
        Ok(
            matches!(status.state, SidecarState::Running | SidecarState::Degraded)
                .then_some(status),
        )
    }
}

/// 探活单次 TCP 连接超时。取值权衡：本地回环连接正常在 1ms 内完成，
/// 500ms 足以覆盖负载高时的抖动；再长会让诊断页明显卡顿。
const PROBE_CONNECT_TIMEOUT_MS: u64 = 500;

/// 恢复路径两次探活之间的最小间隔。
///
/// 检索是高频路径（一章正文可能查十几次），而 sidecar 正常时探活恒定返回健康——
/// 每次检索都做一次 TCP 连接是纯浪费。2 秒的窗口把稳态成本压到接近零
/// （只读一个 Instant），同时保证真崩溃后最迟 2 秒内被下一次检索发现。
///
/// 不做成配置项：它不改变任何可观测行为（只影响发现延迟的上界），
/// 暴露出去只会多一个没人知道该填什么的旋钮。
const RECOVERY_PROBE_MIN_INTERVAL_MS: u64 = 2_000;

/// 单次带超时的 TCP 连通性探测。
///
/// 与 `wait_for_tcp_health` 的区别是**不重试**：那个是启动时等端口就绪的阻塞轮询，
/// 用在诊断路径上会把 sidecar 已死的情况变成一次 startup_timeout_ms 的卡顿。
fn probe_tcp_once(host: &str, port: u16, timeout_ms: u64) -> CoreResult<()> {
    // connect_timeout 需要已解析的 SocketAddr；host 可能是主机名，先解析再连。
    let mut addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| CoreError::External {
            service: "qdrant_sidecar".to_owned(),
            message: format!("cannot resolve {host}:{port}: {error}"),
        })?;
    let address = addresses.next().ok_or_else(|| CoreError::External {
        service: "qdrant_sidecar".to_owned(),
        message: format!("no socket address resolved for {host}:{port}"),
    })?;

    TcpStream::connect_timeout(&address, Duration::from_millis(timeout_ms)).map_err(|error| {
        CoreError::External {
            service: "qdrant_sidecar".to_owned(),
            message: format!("sidecar endpoint {host}:{port} is not reachable: {error}"),
        }
    })?;
    Ok(())
}

/// 判断 degraded 原因是否来自启动期的端口回退。
///
/// 端口回退的降级与运行时连通性无关，探活探通了也不能把它清成 Running——
/// 否则"请求端口被占用"这个用户需要知道的事实会在第一次诊断后凭空消失。
fn is_port_fallback_reason(reason: &Option<String>) -> bool {
    reason
        .as_deref()
        .is_some_and(|reason| reason.starts_with("requested port "))
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

// U116：曾有 `is_port_available(host, port)` = `TcpListener::bind(..).is_ok()`，已删且
// **不要重新加回来**。它 bind 完立刻 drop listener，检查与实际使用之间存在竞态窗口，
// 端口会在这段时间里被别人抢走——这正是 `reserve_available_port` 要一路持有 listener
// 到子进程 spawn 前一刻的原因。留着一个「看起来更简单」的检查函数只会诱导后人退回缺陷版本。

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
