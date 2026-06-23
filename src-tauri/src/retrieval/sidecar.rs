use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::core::{CoreError, CoreResult};
use crate::retrieval::models::{StoreHealth, StoreStatus};

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
    pub fn endpoint(&self, port: u16) -> String {
        format!("http://{}:{port}", self.host)
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidecarState {
    Stopped,
    Running,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortSelection {
    pub port: u16,
    pub reused_requested_port: bool,
}

pub trait SidecarProcessRunner: Send + Sync {
    fn spawn(&self, config: &QdrantSidecarConfig, port: u16) -> CoreResult<Child>;
}

#[derive(Debug, Default)]
pub struct CommandSidecarProcessRunner;

impl SidecarProcessRunner for CommandSidecarProcessRunner {
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

pub struct QdrantSidecarSupervisor<R = CommandSidecarProcessRunner> {
    config: QdrantSidecarConfig,
    runner: R,
    child: Mutex<Option<Child>>,
    status: Mutex<QdrantSidecarStatus>,
}

impl QdrantSidecarSupervisor {
    pub fn new(config: QdrantSidecarConfig) -> Self {
        Self::with_runner(config, CommandSidecarProcessRunner)
    }
}

impl<R> QdrantSidecarSupervisor<R>
where
    R: SidecarProcessRunner,
{
    pub fn with_runner(config: QdrantSidecarConfig, runner: R) -> Self {
        let status = QdrantSidecarStatus::stopped(config.host.clone());
        Self {
            config,
            runner,
            child: Mutex::new(None),
            status: Mutex::new(status),
        }
    }

    pub fn start(&self) -> CoreResult<QdrantSidecarStatus> {
        if let Some(existing) = self.status_if_running()? {
            return Ok(existing);
        }

        let selection = select_available_port(&self.config.host, self.config.requested_port)?;
        let child = match self.runner.spawn(&self.config, selection.port) {
            Ok(child) => child,
            Err(error) => {
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

    pub fn stop(&self) -> CoreResult<QdrantSidecarStatus> {
        if let Some(mut child) = self.child.lock().map_err(lock_error)?.take() {
            child.kill()?;
            let _ = child.wait();
        }

        let status = QdrantSidecarStatus::stopped(self.config.host.clone());
        *self.status.lock().map_err(lock_error)? = status.clone();
        Ok(status)
    }

    pub fn mark_crashed(&self, reason: impl Into<String>) -> CoreResult<QdrantSidecarStatus> {
        let mut status = self.status.lock().map_err(lock_error)?;
        status.state = SidecarState::Unavailable;
        status.reason = Some(reason.into());
        Ok(status.clone())
    }

    pub fn restart(&self) -> CoreResult<QdrantSidecarStatus> {
        self.stop()?;
        self.start()
    }

    pub fn status(&self) -> CoreResult<QdrantSidecarStatus> {
        Ok(self.status.lock().map_err(lock_error)?.clone())
    }

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

    fn status_if_running(&self) -> CoreResult<Option<QdrantSidecarStatus>> {
        let status = self.status()?;
        Ok(
            matches!(status.state, SidecarState::Running | SidecarState::Degraded)
                .then_some(status),
        )
    }
}

pub fn select_available_port(host: &str, requested_port: u16) -> CoreResult<PortSelection> {
    if requested_port == 0 {
        return reserve_ephemeral_port(host).map(|port| PortSelection {
            port,
            reused_requested_port: false,
        });
    }

    if is_port_available(host, requested_port) {
        return Ok(PortSelection {
            port: requested_port,
            reused_requested_port: true,
        });
    }

    reserve_ephemeral_port(host).map(|port| PortSelection {
        port,
        reused_requested_port: false,
    })
}

pub fn is_port_available(host: &str, port: u16) -> bool {
    TcpListener::bind((host, port)).is_ok()
}

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

fn reserve_ephemeral_port(host: &str) -> CoreResult<u16> {
    let listener = TcpListener::bind((host, 0))?;
    Ok(listener.local_addr()?.port())
}

fn lock_error<T>(error: std::sync::PoisonError<T>) -> CoreError {
    CoreError::validation(format!("sidecar supervisor lock poisoned: {error}"))
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
