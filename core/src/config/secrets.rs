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

    fn scoped_key_id(&self, provider_id: &str, domain: &[u8], prefix: &str) -> CoreResult<String> {
        if provider_id.trim().is_empty() {
            return Err(CoreError::validation("provider_id cannot be empty"));
        }
        let mut hasher = Sha256::new();
        hasher.update(b"ariadne-project-credential-v1\0");
        hasher.update(&self.project_identity);
        hasher.update(b"\0");
        hasher.update(domain);
        hasher.update(provider_id.as_bytes());
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

/// 无系统 keychain 时的本地文件 fallback。只用于用户本机 app state，严禁放进项目配置。
#[derive(Clone)]
pub struct LocalFileSecretStore {
    path: PathBuf,
    lock: Arc<RwLock<()>>,
    master_password: Option<Arc<[u8]>>,
}

impl std::fmt::Debug for LocalFileSecretStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalFileSecretStore")
            .field("path", &self.path)
            .field(
                "master_password",
                &self.master_password.as_ref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

impl LocalFileSecretStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Arc::new(RwLock::new(())),
            master_password: std::env::var("ARIADNE_SECRET_MASTER_KEY")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(|value| Arc::<[u8]>::from(value.into_bytes())),
        }
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
            master_password: Some(Arc::<[u8]>::from(
                master_password.expose_secret().as_bytes(),
            )),
        })
    }

    fn master_password(&self) -> CoreResult<&[u8]> {
        self.master_password.as_deref().ok_or_else(|| {
            CoreError::validation(
                "system keychain is unavailable; set a local secret master password before storing provider credentials",
            )
        })
    }

    fn read_values(&self) -> CoreResult<BTreeMap<String, String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(content) if content.trim().is_empty() => Ok(BTreeMap::new()),
            Ok(content) => read_local_secret_file(&content, self.master_password()?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(error) => Err(CoreError::External {
                service: "local_secret_store".to_owned(),
                message: error.to_string(),
            }),
        }
    }

    fn write_values(&self, values: &BTreeMap<String, String>) -> CoreResult<()> {
        let bytes = serde_json::to_vec_pretty(&encrypt_local_secret_values(
            values,
            self.master_password()?,
        )?)
        .map_err(CoreError::from)?;
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

fn read_local_secret_file(
    content: &str,
    master_password: &[u8],
) -> CoreResult<BTreeMap<String, String>> {
    if content.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    match serde_json::from_str::<LocalSecretFile>(content)? {
        LocalSecretFile::Envelope(envelope) => {
            decrypt_local_secret_values(&envelope, master_password)
        }
        LocalSecretFile::LegacyPlaintext(values) => Ok(values),
    }
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
            master_password: None,
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
}
