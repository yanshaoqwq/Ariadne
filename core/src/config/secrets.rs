use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::contracts::{CoreError, CoreResult};

/// 旧项目配置中的密钥引用，仅用于反序列化并触发显式重新绑定。
/// 新配置不得持久化或信任该全局 key id。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretRef {
    pub key_id: String,
}

impl SecretRef {
    /// 创建密钥引用。
    pub fn new(key_id: impl Into<String>) -> Self {
        Self {
            key_id: key_id.into(),
        }
    }
}

/// 内存中的 secret 值，避免误把 String 直接混入配置结构。
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretValue([redacted])")
    }
}

impl SecretValue {
    /// 创建 secret 值。
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// 显式暴露 secret 文本，调用点应避免写入日志。
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

/// 密钥存储抽象，测试和系统 keychain 共用同一接口。
pub trait SecretStore: Send + Sync {
    /// 写入或覆盖密钥。
    fn set_secret(&self, key_id: &str, value: SecretValue) -> CoreResult<()>;
    /// 读取密钥，不存在时返回 None。
    fn get_secret(&self, key_id: &str) -> CoreResult<Option<SecretValue>>;
    /// 删除密钥，不存在时视为成功。
    fn delete_secret(&self, key_id: &str) -> CoreResult<()>;

    /// U118：本存储当前的凭据保护状态，供设置页与诊断展示。
    ///
    /// 默认 `Managed`——系统钥匙链与内存存储都由宿主保管，没有「主密码」这一层，
    /// 也没有明文落盘风险。只有 `LocalFileSecretStore` 需要覆写。
    fn protection_status(&self) -> SecretProtectionStatus {
        SecretProtectionStatus::Managed
    }

    /// U118：运行时设置本地主密码。
    ///
    /// 默认返回 `Err`：对不需要主密码的存储，静默成功会让 UI 误以为已加密。
    fn set_master_password(&self, _master_password: SecretValue) -> CoreResult<()> {
        Err(CoreError::validation(
            "this secret store is managed by the host and takes no master password",
        ))
    }

    /// U118：用户显式接受明文存储。默认拒绝，理由同上。
    fn allow_unprotected_storage(&self) -> CoreResult<()> {
        Err(CoreError::validation(
            "this secret store is managed by the host and cannot be switched to plain text",
        ))
    }
}

/// U118：凭据保护状态，是设置页与诊断的唯一真相源。
///
/// 之所以要把 `Unprotected` 单列而不是并进 `Locked`：用户当时同意了明文，
/// 三个月后未必记得。诊断必须能持续把这件事说出来，否则「同意过」就变成了
/// 一次性的、再也看不见的决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretProtectionStatus {
    /// 由宿主保管（系统钥匙链 / 进程内存），无需主密码。
    Managed,
    /// 已设主密码，凭据以密文落盘。
    Encrypted,
    /// 用户显式接受明文落盘。
    Unprotected,
    /// 既无主密码也无明文许可——此状态下保存凭据会失败。
    Locked,
}

/// 把通用 SecretStore 收束为当前项目的 Provider 凭据能力。
///
/// 项目配置只能提供 provider id，不能选择全局 key id；项目身份使用
/// canonical path 的无损平台字节参与 SHA-256 派生，移动/导入后必须重新绑定。
pub struct ProjectCredentialScope<'a> {
    secrets: &'a dyn SecretStore,
    project_identity: Vec<u8>,
}

impl<'a> ProjectCredentialScope<'a> {
    /// 为已存在的项目根创建可信凭据作用域。
    pub fn new(project_root: &Path, secrets: &'a dyn SecretStore) -> CoreResult<Self> {
        let canonical_root = project_root.canonicalize()?;
        Ok(Self {
            secrets,
            project_identity: project_path_identity_bytes(&canonical_root),
        })
    }

    /// 读取当前项目指定 Provider 的凭据。
    pub fn get_provider_secret(&self, provider_id: &str) -> CoreResult<Option<SecretValue>> {
        self.secrets.get_secret(&self.provider_key_id(provider_id)?)
    }

    /// 返回项目 Provider 凭据代次。代次本身保存在 SecretStore 中，不进入项目配置；
    /// 工作流只持久化该不透明标识，用于恢复时拒绝静默采用替换后的凭据。
    pub fn provider_secret_generation(&self, provider_id: &str) -> CoreResult<String> {
        let generation_key = self.provider_generation_key_id(provider_id)?;
        if let Some(generation) = self.secrets.get_secret(&generation_key)? {
            let generation = generation.expose_secret().trim();
            if !generation.is_empty() {
                return Ok(generation.to_owned());
            }
        }
        let generation = new_secret_generation();
        self.secrets
            .set_secret(&generation_key, SecretValue::new(generation.clone()))?;
        Ok(generation)
    }

    /// 写入当前项目指定 Provider 的凭据。
    pub fn set_provider_secret(&self, provider_id: &str, value: SecretValue) -> CoreResult<()> {
        // 先推进代次再写凭据：若第二步失败，旧运行会因代次不匹配安全失败。
        self.secrets.set_secret(
            &self.provider_generation_key_id(provider_id)?,
            SecretValue::new(new_secret_generation()),
        )?;
        self.secrets
            .set_secret(&self.provider_key_id(provider_id)?, value)
    }

    /// 删除当前项目指定 Provider 的凭据。
    pub fn delete_provider_secret(&self, provider_id: &str) -> CoreResult<()> {
        self.secrets.set_secret(
            &self.provider_generation_key_id(provider_id)?,
            SecretValue::new(new_secret_generation()),
        )?;
        self.secrets
            .delete_secret(&self.provider_key_id(provider_id)?)
    }

    /// 读取与规范化外部 Qdrant 端点绑定的 API key。端点参与派生，配置改址后不会复用旧密钥。
    pub fn get_external_qdrant_secret(
        &self,
        endpoint_identity: &str,
    ) -> CoreResult<Option<SecretValue>> {
        self.secrets
            .get_secret(&self.external_qdrant_key_id(endpoint_identity)?)
    }

    /// 写入与外部 Qdrant 端点绑定的 API key。
    pub fn set_external_qdrant_secret(
        &self,
        endpoint_identity: &str,
        value: SecretValue,
    ) -> CoreResult<()> {
        self.secrets.set_secret(
            &self.external_qdrant_generation_key_id(endpoint_identity)?,
            SecretValue::new(new_secret_generation()),
        )?;
        self.secrets
            .set_secret(&self.external_qdrant_key_id(endpoint_identity)?, value)
    }

    /// 删除与外部 Qdrant 端点绑定的 API key。
    pub fn delete_external_qdrant_secret(&self, endpoint_identity: &str) -> CoreResult<()> {
        self.secrets.set_secret(
            &self.external_qdrant_generation_key_id(endpoint_identity)?,
            SecretValue::new(new_secret_generation()),
        )?;
        self.secrets
            .delete_secret(&self.external_qdrant_key_id(endpoint_identity)?)
    }

    /// 返回端点凭据代次，使运行时在密钥替换后拒绝复用旧请求客户端。
    pub fn external_qdrant_secret_generation(&self, endpoint_identity: &str) -> CoreResult<String> {
        let generation_key = self.external_qdrant_generation_key_id(endpoint_identity)?;
        if let Some(generation) = self.secrets.get_secret(&generation_key)? {
            let generation = generation.expose_secret().trim();
            if !generation.is_empty() {
                return Ok(generation.to_owned());
            }
        }
        let generation = new_secret_generation();
        self.secrets
            .set_secret(&generation_key, SecretValue::new(generation.clone()))?;
        Ok(generation)
    }

    fn provider_key_id(&self, provider_id: &str) -> CoreResult<String> {
        self.scoped_key_id(provider_id, b"provider\0", "ariadne-credential-v1-")
    }

    fn provider_generation_key_id(&self, provider_id: &str) -> CoreResult<String> {
        self.scoped_key_id(
            provider_id,
            b"provider-generation\0",
            "ariadne-credential-generation-v1-",
        )
    }

    fn external_qdrant_key_id(&self, endpoint_identity: &str) -> CoreResult<String> {
        self.scoped_key_id(
            endpoint_identity,
            b"external-qdrant\0",
            "ariadne-qdrant-credential-v1-",
        )
    }

    fn external_qdrant_generation_key_id(&self, endpoint_identity: &str) -> CoreResult<String> {
        self.scoped_key_id(
            endpoint_identity,
            b"external-qdrant-generation\0",
            "ariadne-qdrant-credential-generation-v1-",
        )
    }

    fn scoped_key_id(&self, identity: &str, domain: &[u8], prefix: &str) -> CoreResult<String> {
        if identity.trim().is_empty() {
            return Err(CoreError::validation("credential identity cannot be empty"));
        }
        let mut hasher = Sha256::new();
        hasher.update(b"ariadne-project-credential-v1\0");
        hasher.update(&self.project_identity);
        hasher.update(b"\0");
        hasher.update(domain);
        hasher.update(identity.as_bytes());
        let digest = hasher.finalize();
        let mut encoded = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use std::fmt::Write;
            write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
        }
        Ok(format!("{prefix}{encoded}"))
    }
}

fn new_secret_generation() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{timestamp:032x}-{sequence:016x}")
}

#[cfg(unix)]
fn project_path_identity_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    let mut bytes = b"unix\0".to_vec();
    bytes.extend_from_slice(path.as_os_str().as_bytes());
    bytes
}

#[cfg(windows)]
fn project_path_identity_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    let mut bytes = b"windows\0".to_vec();
    for unit in path.as_os_str().encode_wide() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

/// 测试用内存密钥存储。
#[derive(Debug, Clone, Default)]
pub struct MemorySecretStore {
    values: Arc<RwLock<BTreeMap<String, SecretValue>>>,
}

impl SecretStore for MemorySecretStore {
    /// 写入内存密钥。
    fn set_secret(&self, key_id: &str, value: SecretValue) -> CoreResult<()> {
        if key_id.trim().is_empty() {
            return Err(CoreError::validation("key_id cannot be empty"));
        }

        let mut values = self
            .values
            .write()
            .map_err(|_| CoreError::validation("secret store lock poisoned"))?;
        values.insert(key_id.to_owned(), value);
        Ok(())
    }

    /// 从内存读取密钥。
    fn get_secret(&self, key_id: &str) -> CoreResult<Option<SecretValue>> {
        let values = self
            .values
            .read()
            .map_err(|_| CoreError::validation("secret store lock poisoned"))?;
        Ok(values.get(key_id).cloned())
    }

    /// 从内存删除密钥。
    fn delete_secret(&self, key_id: &str) -> CoreResult<()> {
        let mut values = self
            .values
            .write()
            .map_err(|_| CoreError::validation("secret store lock poisoned"))?;
        values.remove(key_id);
        Ok(())
    }
}

/// U118：本地密钥文件的保护方式。
///
/// 之所以要有 `Unprotected` 这个**显式**变体，而不是让「没有主密码」隐含地退化为
/// 明文：明文存储必须是用户知情下的选择。若靠缺省行为静默落盘，用户不会知道自己的
/// API Key 正躺在磁盘上——而这类事一旦发生，用户是最后一个知道的人。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalSecretProtection {
    /// 主密码加密（Argon2id + ChaCha20-Poly1305）。
    Encrypted,
    /// 用户显式接受的明文存储；仅限本机 app state 目录，文件权限 0600。
    Unprotected,
}

/// 无系统 keychain 时的本地文件 fallback。只用于用户本机 app state，严禁放进项目配置。
#[derive(Clone)]
pub struct LocalFileSecretStore {
    path: PathBuf,
    lock: Arc<RwLock<()>>,
    /// U118：保护状态必须**运行时可变**。
    ///
    /// `SecretStore` 的三个方法都是 `&self`，而 `AriadneAppState.secret_store` 是
    /// 不可变的 `Arc<dyn SecretStore>`——若把主密码做成不可变字段，用户就只能
    /// 「退出应用 → 改环境变量 → 重启」，对 GUI 用户等于没修。故收进 RwLock，
    /// 让 `set_master_password` / `allow_unprotected` 在进程内当场生效。
    ///
    /// 用独立锁而非复用文件锁 `lock`：文件锁保护的是**读改写序列**，
    /// 若与状态共用，解锁期间的任何读都会被阻塞在同一把锁上。
    state: Arc<RwLock<LocalSecretProtectionState>>,
}

/// 本地密钥文件的进程内保护状态。
#[derive(Debug, Default, Clone)]
struct LocalSecretProtectionState {
    master_password: Option<Arc<[u8]>>,
    /// 无主密码时是否已获得用户明确许可以明文存储。
    ///
    /// 默认 `false`：这样「忘了接线主密码流程」的后果是**保存失败并报错**，
    /// 而不是静默明文落盘。安全默认值应当让疏漏表现为可见的失败。
    allow_unprotected: bool,
}

impl std::fmt::Debug for LocalFileSecretStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalFileSecretStore")
            .field("path", &self.path)
            .field(
                "master_password",
                &self
                    .protection_state()
                    .and_then(|state| state.master_password.as_ref().map(|_| "[redacted]")),
            )
            .finish()
    }
}

impl LocalFileSecretStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Arc::new(RwLock::new(())),
            state: Arc::new(RwLock::new(LocalSecretProtectionState {
                master_password: std::env::var("ARIADNE_SECRET_MASTER_KEY")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| Arc::<[u8]>::from(value.into_bytes())),
                allow_unprotected: false,
            })),
        }
    }

    /// U118：用户显式接受明文存储后构造。
    ///
    /// 与 `with_master_password` 并列而非互斥参数，是为了让调用点读起来就能看出
    /// 选了哪种保护方式——`Option<SecretValue>` 那种写法会把「没密码」和
    /// 「同意明文」混成同一个 `None`，正是这条缺陷最初的成因。
    pub fn unprotected(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Arc::new(RwLock::new(())),
            state: Arc::new(RwLock::new(LocalSecretProtectionState {
                master_password: None,
                allow_unprotected: true,
            })),
        }
    }

    /// 返回当前生效的保护方式，供诊断与 UI 展示。
    pub fn protection(&self) -> LocalSecretProtection {
        match self.protection_state() {
            Some(state) if state.master_password.is_some() => LocalSecretProtection::Encrypted,
            _ => LocalSecretProtection::Unprotected,
        }
    }

    /// 读取保护状态快照；锁中毒时返回 None，由调用方决定降级方式。
    fn protection_state(&self) -> Option<LocalSecretProtectionState> {
        self.state.read().ok().map(|state| state.clone())
    }

    /// U118：运行时设置主密码并立即生效（无需重启）。
    ///
    /// 已存在的明文文件不在此刻重写——重写发生在下一次 `set_secret`，
    /// 那是唯一一个本就要整体落盘的时机。在这里顺手重写等于把「设密码」
    /// 变成一次隐式的全量写盘，失败时用户会既没设上密码、又丢了原文件。
    pub fn set_master_password(&self, master_password: SecretValue) -> CoreResult<()> {
        if master_password.expose_secret().trim().is_empty() {
            return Err(CoreError::validation(
                "local secret master password cannot be empty",
            ));
        }
        let mut state = self.state.write().map_err(|_| {
            CoreError::validation("local secret protection state lock poisoned")
        })?;
        state.master_password = Some(Arc::<[u8]>::from(
            master_password.expose_secret().as_bytes(),
        ));
        // 设了密码就不再允许明文：两者并存会让「到底存成了什么」取决于调用顺序。
        state.allow_unprotected = false;
        Ok(())
    }

    /// U118：用户显式接受明文存储。
    pub fn allow_unprotected(&self) -> CoreResult<()> {
        let mut state = self.state.write().map_err(|_| {
            CoreError::validation("local secret protection state lock poisoned")
        })?;
        state.allow_unprotected = true;
        Ok(())
    }

    /// 无系统 keychain 时由上层主密码流程显式注入。密码只保存在进程内存中。
    pub fn with_master_password(
        path: impl Into<PathBuf>,
        master_password: SecretValue,
    ) -> CoreResult<Self> {
        if master_password.expose_secret().trim().is_empty() {
            return Err(CoreError::validation(
                "local secret master password cannot be empty",
            ));
        }
        Ok(Self {
            path: path.into(),
            lock: Arc::new(RwLock::new(())),
            state: Arc::new(RwLock::new(LocalSecretProtectionState {
                master_password: Some(Arc::<[u8]>::from(
                    master_password.expose_secret().as_bytes(),
                )),
                allow_unprotected: false,
            })),
        })
    }

    /// 取主密码；既无密码又未获明文许可时 fail-loud。
    ///
    /// U118：**错误信息不得指向产品里不存在的操作**。原文案让用户去
    /// 「set a local secret master password」，而那个操作当时既无 IPC 命令也无 UI，
    /// 用户照着做只会撞墙。现在两条出路都是真实可达的命令，故直接写出命令名。
    fn master_password(&self) -> CoreResult<Arc<[u8]>> {
        self.protection_state()
            .and_then(|state| state.master_password.clone())
            .ok_or_else(|| self.locked_error())
    }

    /// 「未解锁且未许可明文」的统一错误，两处共用避免文案漂移。
    fn locked_error(&self) -> CoreError {
        CoreError::validation(
            "local secret store is locked: call set_local_secret_master_password to encrypt \
             credentials, or allow_unprotected_local_secrets to store them in plain text",
        )
    }

    fn read_values(&self) -> CoreResult<BTreeMap<String, String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(content) if content.trim().is_empty() => Ok(BTreeMap::new()),
            Ok(content) => self.decode_values(&content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(error) => Err(CoreError::External {
                service: "local_secret_store".to_owned(),
                message: error.to_string(),
            }),
        }
    }

    /// 按文件自述的形态解码，而不是按当前进程的模式假定。
    ///
    /// 关键点：**是否需要主密码由文件说了算**。用户可能先用明文存了几个 key，
    /// 之后再设主密码；也可能反过来。若按进程模式去猜，就会出现
    /// 「加密文件用明文路径读 → 报格式错」这类误导性失败。
    ///
    /// **但锁定的 store 不得读取意外出现的明文文件**：那是安全护栏。若有人往
    /// app state 目录丢了一个明文 secrets.json（迁移/误操作/攻击），锁定的 store
    /// 应当拒绝，而非静默读出来交给调用方——让调用方去检查保护模式显然太晚了。
    fn decode_values(&self, content: &str) -> CoreResult<BTreeMap<String, String>> {
        let protection = self.protection_state().unwrap_or_default();
        match serde_json::from_str::<LocalSecretFile>(content)? {
            LocalSecretFile::Envelope(envelope) => {
                decrypt_local_secret_values(&envelope, &self.master_password()?)
            }
            LocalSecretFile::LegacyPlaintext(values) => {
                // 明文文件可被两类 store 读取：
                // - 已获明文许可的（用户选了不加密）；
                // - **持有主密码的**——这是历史明文文件的迁移路径：读出后下一次写入
                //   即以密文重写（回归见 `..._reads_legacy_plaintext_and_rewrites_encrypted`）。
                //
                // 只有「既没密码、也没许可」的锁定 store 才拒绝：若有人往 app state
                // 目录丢了明文 secrets.json（迁移/误操作/攻击），静默读出来交给调用方
                // 等于让护栏形同虚设。
                if protection.master_password.is_none() && !protection.allow_unprotected {
                    return Err(self.locked_error());
                }
                Ok(values)
            }
        }
    }

    fn write_values(&self, values: &BTreeMap<String, String>) -> CoreResult<()> {
        let protection = self.protection_state().unwrap_or_default();
        let bytes = match protection.master_password.as_deref() {
            Some(password) => {
                serde_json::to_vec_pretty(&encrypt_local_secret_values(values, password)?)
                    .map_err(CoreError::from)?
            }
            // 无主密码时**必须**已获显式许可；否则报错而不是静默明文落盘。
            None if protection.allow_unprotected => {
                serde_json::to_vec_pretty(values).map_err(CoreError::from)?
            }
            None => return Err(self.locked_error()),
        };
        // D4：密钥文件与文档正文共用 atomic_write（临时文件 + rename），避免覆盖写半文件。
        crate::config::store::atomic_write(&self.path, &bytes).map_err(|error| {
            CoreError::External {
                service: "local_secret_store".to_owned(),
                message: error.to_string(),
            }
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))
                .map_err(io_secret_error)?;
        }
        Ok(())
    }
}

impl SecretStore for LocalFileSecretStore {
    fn protection_status(&self) -> SecretProtectionStatus {
        let Some(state) = self.protection_state() else {
            // 锁中毒时按最保守的状态上报：宁可提示用户「已锁定」，
            // 也不要报成 Encrypted 让人以为凭据安全地加密着。
            return SecretProtectionStatus::Locked;
        };
        match (&state.master_password, state.allow_unprotected) {
            (Some(_), _) => SecretProtectionStatus::Encrypted,
            (None, true) => SecretProtectionStatus::Unprotected,
            (None, false) => SecretProtectionStatus::Locked,
        }
    }

    fn set_master_password(&self, master_password: SecretValue) -> CoreResult<()> {
        LocalFileSecretStore::set_master_password(self, master_password)
    }

    fn allow_unprotected_storage(&self) -> CoreResult<()> {
        self.allow_unprotected()
    }

    fn set_secret(&self, key_id: &str, value: SecretValue) -> CoreResult<()> {
        if key_id.trim().is_empty() {
            return Err(CoreError::validation("key_id cannot be empty"));
        }
        let _guard = self
            .lock
            .write()
            .map_err(|_| CoreError::validation("secret store lock poisoned"))?;
        let mut values = self.read_values()?;
        values.insert(key_id.to_owned(), value.expose_secret().to_owned());
        self.write_values(&values)
    }

    fn get_secret(&self, key_id: &str) -> CoreResult<Option<SecretValue>> {
        let _guard = self
            .lock
            .read()
            .map_err(|_| CoreError::validation("secret store lock poisoned"))?;
        Ok(self
            .read_values()?
            .get(key_id)
            .cloned()
            .map(SecretValue::new))
    }

    fn delete_secret(&self, key_id: &str) -> CoreResult<()> {
        let _guard = self
            .lock
            .write()
            .map_err(|_| CoreError::validation("secret store lock poisoned"))?;
        let mut values = self.read_values()?;
        values.remove(key_id);
        self.write_values(&values)
    }
}

fn io_secret_error(error: std::io::Error) -> CoreError {
    CoreError::External {
        service: "local_secret_store".to_owned(),
        message: error.to_string(),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct LocalSecretEnvelope {
    version: u8,
    cipher: String,
    kdf: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    salt_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    memory_kib: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parallelism: Option<u32>,
    nonce_hex: String,
    ciphertext_hex: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum LocalSecretFile {
    Envelope(LocalSecretEnvelope),
    LegacyPlaintext(BTreeMap<String, String>),
}


fn encrypt_local_secret_values(
    values: &BTreeMap<String, String>,
    master_password: &[u8],
) -> CoreResult<LocalSecretEnvelope> {
    const MEMORY_KIB: u32 = 19 * 1024;
    const ITERATIONS: u32 = 3;
    const PARALLELISM: u32 = 1;
    let mut salt_bytes = [0u8; 16];
    getrandom::getrandom(&mut salt_bytes).map_err(local_secret_random_error)?;
    let key_bytes = derive_argon2id_key(
        master_password,
        &salt_bytes,
        MEMORY_KIB,
        ITERATIONS,
        PARALLELISM,
    )?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key_bytes));
    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes).map_err(local_secret_random_error)?;
    let plaintext = serde_json::to_vec(values)?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
        .map_err(local_secret_crypto_error)?;
    Ok(LocalSecretEnvelope {
        version: 3,
        cipher: "chacha20poly1305".to_owned(),
        kdf: "argon2id".to_owned(),
        salt_hex: Some(encode_hex(&salt_bytes)),
        memory_kib: Some(MEMORY_KIB),
        iterations: Some(ITERATIONS),
        parallelism: Some(PARALLELISM),
        nonce_hex: encode_hex(&nonce_bytes),
        ciphertext_hex: encode_hex(&ciphertext),
    })
}

fn decrypt_local_secret_values(
    envelope: &LocalSecretEnvelope,
    master_password: &[u8],
) -> CoreResult<BTreeMap<String, String>> {
    if envelope.cipher != "chacha20poly1305" {
        return Err(CoreError::validation(format!(
            "unsupported local secret cipher {}",
            envelope.cipher
        )));
    }
    let nonce = decode_hex(&envelope.nonce_hex)?;
    if nonce.len() != 12 {
        return Err(CoreError::validation("local secret nonce must be 12 bytes"));
    }
    let ciphertext = decode_hex(&envelope.ciphertext_hex)?;
    let key_bytes = match envelope.version {
        3 => {
            if envelope.kdf != "argon2id" {
                return Err(CoreError::validation(format!(
                    "unsupported local secret kdf {}",
                    envelope.kdf
                )));
            }
            let salt = decode_hex(
                envelope
                    .salt_hex
                    .as_deref()
                    .ok_or_else(|| CoreError::validation("local secret salt is missing"))?,
            )?;
            derive_argon2id_key(
                master_password,
                &salt,
                envelope
                    .memory_kib
                    .ok_or_else(|| CoreError::validation("local secret memory cost is missing"))?,
                envelope.iterations.ok_or_else(|| {
                    CoreError::validation("local secret iteration count is missing")
                })?,
                envelope
                    .parallelism
                    .ok_or_else(|| CoreError::validation("local secret parallelism is missing"))?,
            )?
        }
        2 => derive_legacy_v2_key(&envelope.kdf, master_password)?,
        other => {
            return Err(CoreError::validation(format!(
                "unsupported local secret store version {other}",
            )))
        }
    };
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key_bytes));
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(local_secret_crypto_error)?;
    serde_json::from_slice(&plaintext).map_err(CoreError::from)
}

fn derive_argon2id_key(
    master_password: &[u8],
    salt: &[u8],
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
) -> CoreResult<[u8; 32]> {
    let params = Params::new(memory_kib, iterations, parallelism, Some(32))
        .map_err(local_secret_kdf_error)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(master_password, salt, &mut key)
        .map_err(local_secret_kdf_error)?;
    Ok(key)
}

fn derive_legacy_v2_key(kdf: &str, master_password: &[u8]) -> CoreResult<[u8; 32]> {
    derive_legacy_v2_key_with_machine_migration(
        kdf,
        master_password,
        std::env::var("ARIADNE_ALLOW_LEGACY_MACHINE_SECRET_MIGRATION").as_deref() == Ok("1"),
    )
}

fn derive_legacy_v2_key_with_machine_migration(
    kdf: &str,
    master_password: &[u8],
    allow_machine_migration: bool,
) -> CoreResult<[u8; 32]> {
    let mut hasher = Sha256::new();
    hasher.update(b"ariadne-local-secret-store-v2");
    if kdf == "env_master_key_sha256" {
        hasher.update(b"\0env-master-key\0");
        hasher.update(master_password);
        return Ok(digest_to_key(hasher.finalize()));
    }
    if kdf != "machine_bound_sha256" || !allow_machine_migration {
        return Err(CoreError::validation(
            "legacy machine-bound secret store is disabled; explicitly enable one-time migration and re-save with a master password",
        ));
    }
    hasher.update(b"\0machine-bound-fallback\0");
    for path in [
        "/etc/machine-id",
        "/var/lib/dbus/machine-id",
        "/etc/hostname",
    ] {
        if let Ok(value) = std::fs::read_to_string(path) {
            hasher.update(path.as_bytes());
            hasher.update(b"\0");
            hasher.update(value.trim().as_bytes());
            hasher.update(b"\0");
        }
    }
    for name in ["USER", "USERNAME", "HOME", "APPDATA"] {
        if let Ok(value) = std::env::var(name) {
            hasher.update(name.as_bytes());
            hasher.update(b"\0");
            hasher.update(value.as_bytes());
            hasher.update(b"\0");
        }
    }
    Ok(digest_to_key(hasher.finalize()))
}

fn digest_to_key(digest: impl AsRef<[u8]>) -> [u8; 32] {
    let mut key = [0u8; 32];
    key.copy_from_slice(digest.as_ref());
    key
}

fn local_secret_random_error(error: getrandom::Error) -> CoreError {
    CoreError::External {
        service: "local_secret_store".to_owned(),
        message: format!("failed to generate secret encryption randomness: {error}"),
    }
}

fn local_secret_kdf_error(error: argon2::Error) -> CoreError {
    CoreError::External {
        service: "local_secret_store".to_owned(),
        message: format!("local secret key derivation failed: {error}"),
    }
}

fn local_secret_crypto_error(error: chacha20poly1305::Error) -> CoreError {
    CoreError::External {
        service: "local_secret_store".to_owned(),
        message: format!("local secret encryption failed: {error}"),
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_hex(value: &str) -> CoreResult<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(CoreError::validation("hex value must have even length"));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    let raw = value.as_bytes();
    for index in (0..raw.len()).step_by(2) {
        let high = hex_digit(raw[index])?;
        let low = hex_digit(raw[index + 1])?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn hex_digit(byte: u8) -> CoreResult<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(CoreError::validation("invalid hex digit")),
    }
}

#[cfg(feature = "system-keychain")]
/// 系统 keychain 密钥存储。
#[derive(Debug, Clone)]
pub struct SystemKeychainSecretStore {
    service: String,
}

#[cfg(feature = "system-keychain")]
impl SystemKeychainSecretStore {
    /// 创建指定 service 名称的系统 keychain 存储。
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// 获取 keyring 条目，并统一校验 key id。
    fn entry(&self, key_id: &str) -> CoreResult<keyring::Entry> {
        if key_id.trim().is_empty() {
            return Err(CoreError::validation("key_id cannot be empty"));
        }

        keyring::Entry::new(&self.service, key_id).map_err(keyring_error)
    }
}

#[cfg(feature = "system-keychain")]
impl Default for SystemKeychainSecretStore {
    /// 使用项目默认 service 名称创建系统 keychain 存储。
    /// 旧版使用 "literature-agent"，迁移时需尝试读取旧 service 名下的密钥。
    fn default() -> Self {
        Self::new("ariadne")
    }
}

#[cfg(feature = "system-keychain")]
impl SecretStore for SystemKeychainSecretStore {
    /// 写入系统 keychain。
    fn set_secret(&self, key_id: &str, value: SecretValue) -> CoreResult<()> {
        self.entry(key_id)?
            .set_password(value.expose_secret())
            .map_err(keyring_error)
    }

    /// 从系统 keychain 读取密钥。
    fn get_secret(&self, key_id: &str) -> CoreResult<Option<SecretValue>> {
        match self.entry(key_id)?.get_password() {
            Ok(value) => Ok(Some(SecretValue::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keyring_error(error)),
        }
    }

    /// 从系统 keychain 删除密钥。
    fn delete_secret(&self, key_id: &str) -> CoreResult<()> {
        match self.entry(key_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keyring_error(error)),
        }
    }
}

#[cfg(feature = "system-keychain")]
/// 将 keyring 错误转换成统一外部服务错误。
fn keyring_error(error: keyring::Error) -> CoreError {
    CoreError::External {
        service: "system_keychain".to_owned(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_secret_store_round_trips_by_key_id() {
        let store = MemorySecretStore::default();
        store
            .set_secret("openai-main", SecretValue::new("sk-secret"))
            .unwrap();

        let secret = store.get_secret("openai-main").unwrap().unwrap();
        assert_eq!(secret.expose_secret(), "sk-secret");
    }

    #[test]
    fn secret_debug_output_redacts_values_and_master_password() {
        let secret = SecretValue::new("sk-never-log-this");
        assert_eq!(format!("{secret:?}"), "SecretValue([redacted])");

        let store = LocalFileSecretStore::with_master_password(
            "secrets.json",
            SecretValue::new("master-never-log-this"),
        )
        .unwrap();
        let debug = format!("{store:?}");
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains("master-never-log-this"));
    }

    #[test]
    fn local_file_secret_store_without_master_password_refuses_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("secrets.json");
        std::fs::write(&path, r#"{"legacy":"must-not-open"}"#).unwrap();
        let before = std::fs::read(&path).unwrap();
        let store = LocalFileSecretStore {
            path: path.clone(),
            lock: Arc::new(RwLock::new(())),
            state: Arc::new(RwLock::new(LocalSecretProtectionState::default())),
        };

        assert!(store.get_secret("legacy").is_err());
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[test]
    fn legacy_machine_bound_key_requires_explicit_migration_mode() {
        assert!(derive_legacy_v2_key_with_machine_migration(
            "machine_bound_sha256",
            b"unused",
            false,
        )
        .is_err());
        assert!(derive_legacy_v2_key_with_machine_migration(
            "machine_bound_sha256",
            b"unused",
            true,
        )
        .is_ok());
    }

    #[test]
    fn local_file_secret_store_persists_between_instances() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("secrets.json");
        let store = LocalFileSecretStore::with_master_password(
            &path,
            SecretValue::new("correct horse battery staple"),
        )
        .unwrap();
        store
            .set_secret("openai-main", SecretValue::new("sk-secret"))
            .unwrap();

        let reloaded = LocalFileSecretStore::with_master_password(
            &path,
            SecretValue::new("correct horse battery staple"),
        )
        .unwrap();
        let secret = reloaded.get_secret("openai-main").unwrap().unwrap();
        assert_eq!(secret.expose_secret(), "sk-secret");

        let file = std::fs::read_to_string(&path).unwrap();
        assert!(file.contains("chacha20poly1305"));
        assert!(file.contains("argon2id"));
        assert!(!file.contains("sk-secret"));
    }

    #[test]
    fn local_file_secret_store_wrong_password_does_not_modify_ciphertext() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("secrets.json");
        LocalFileSecretStore::with_master_password(&path, SecretValue::new("correct-password"))
            .unwrap()
            .set_secret("openai-main", SecretValue::new("sk-secret"))
            .unwrap();
        let before = std::fs::read(&path).unwrap();

        let wrong =
            LocalFileSecretStore::with_master_password(&path, SecretValue::new("wrong-password"))
                .unwrap();
        assert!(wrong.get_secret("openai-main").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn local_file_secret_store_reads_legacy_plaintext_and_rewrites_encrypted() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("secrets.json");
        std::fs::write(&path, r#"{"legacy":"old-secret"}"#).unwrap();

        let store = LocalFileSecretStore::with_master_password(
            &path,
            SecretValue::new("migration-password"),
        )
        .unwrap();
        let secret = store.get_secret("legacy").unwrap().unwrap();
        assert_eq!(secret.expose_secret(), "old-secret");
        store
            .set_secret("new", SecretValue::new("new-secret"))
            .unwrap();

        let file = std::fs::read_to_string(&path).unwrap();
        assert!(file.contains("chacha20poly1305"));
        assert!(!file.contains("old-secret"));
        assert!(!file.contains("new-secret"));
    }

    /// U118：无密码、无许可的 store **必须拒绝写入**，而不是静默明文落盘。
    ///
    /// 这是整条修复的安全底线。若默认行为是明文，任何一处忘了接线保护流程，
    /// 用户的 API Key 就会悄无声息地躺在磁盘上——而用户是最后一个知道的人。
    #[test]
    fn locked_store_refuses_to_write_instead_of_falling_back_to_plaintext() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("secrets.json");
        let store = LocalFileSecretStore {
            path: path.clone(),
            lock: Arc::new(RwLock::new(())),
            state: Arc::new(RwLock::new(LocalSecretProtectionState::default())),
        };

        let error = store
            .set_secret("provider", SecretValue::new("sk-should-not-land"))
            .expect_err("锁定状态下写入必须失败");

        assert!(
            !path.exists(),
            "写入被拒时不得留下任何文件，否则半个明文文件比报错更危险"
        );
        // 错误必须指向**真实存在**的补救操作：这正是 U118 原缺陷的核心——
        // 旧文案让用户去做一个产品里根本没有的动作。
        let message = error.to_string();
        assert!(
            message.contains("set_local_secret_master_password")
                && message.contains("allow_unprotected_local_secrets"),
            "错误应给出两条真实可达的出路，实际：{message}"
        );
    }

    /// U118：运行时设主密码后立即生效，无需重启进程。
    ///
    /// 方案 1 的核心诉求。若只能靠启动时的环境变量，GUI 用户就得
    /// 「退出应用 → 改配置 → 重启」，对他们等于没修。
    #[test]
    fn master_password_set_at_runtime_takes_effect_immediately() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("secrets.json");
        let store = LocalFileSecretStore {
            path: path.clone(),
            lock: Arc::new(RwLock::new(())),
            state: Arc::new(RwLock::new(LocalSecretProtectionState::default())),
        };
        assert!(store.set_secret("k", SecretValue::new("v")).is_err());

        store
            .set_master_password(SecretValue::new("runtime-password"))
            .unwrap();

        store.set_secret("k", SecretValue::new("v")).unwrap();
        assert_eq!(store.protection(), LocalSecretProtection::Encrypted);
        let file = std::fs::read_to_string(&path).unwrap();
        assert!(file.contains("chacha20poly1305"), "应当以密文落盘");
        assert!(!file.contains("\"v\""), "明文值不得出现在文件里");
    }

    /// U118：用户显式接受明文后才允许明文落盘。
    #[test]
    fn unprotected_mode_requires_explicit_opt_in() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("secrets.json");
        let store = LocalFileSecretStore::unprotected(&path);

        store.set_secret("k", SecretValue::new("plain-value")).unwrap();

        assert_eq!(store.protection(), LocalSecretProtection::Unprotected);
        let file = std::fs::read_to_string(&path).unwrap();
        assert!(
            file.contains("plain-value"),
            "用户选择明文时就应当是明文——含糊其辞的「半加密」只会给人虚假的安全感"
        );
        assert_eq!(
            store.get_secret("k").unwrap().unwrap().expose_secret(),
            "plain-value"
        );
    }

    /// U118：设了主密码就不再允许明文，避免「存成什么取决于调用顺序」。
    #[test]
    fn setting_master_password_revokes_unprotected_permission() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("secrets.json");
        let store = LocalFileSecretStore::unprotected(&path);
        store
            .set_master_password(SecretValue::new("now-encrypted"))
            .unwrap();

        store.set_secret("k", SecretValue::new("v")).unwrap();

        assert_eq!(store.protection(), LocalSecretProtection::Encrypted);
        let file = std::fs::read_to_string(&path).unwrap();
        assert!(!file.contains("\"v\""), "设过密码后不得再明文落盘");
    }
}
