use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::contracts::{CoreError, CoreResult};

/// 项目配置中保存的密钥引用，只包含 key id，不包含真实 secret。
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretValue(String);

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
#[derive(Debug, Clone)]
pub struct LocalFileSecretStore {
    path: PathBuf,
    lock: Arc<RwLock<()>>,
}

impl LocalFileSecretStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Arc::new(RwLock::new(())),
        }
    }

    fn read_values(&self) -> CoreResult<BTreeMap<String, String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(content) => read_local_secret_file(&content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(error) => Err(CoreError::External {
                service: "local_secret_store".to_owned(),
                message: error.to_string(),
            }),
        }
    }

    fn write_values(&self, values: &BTreeMap<String, String>) -> CoreResult<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(io_secret_error)?;
        }
        std::fs::write(
            &self.path,
            serde_json::to_string_pretty(&encrypt_local_secret_values(values)?)
                .map_err(CoreError::from)?,
        )
        .map_err(io_secret_error)?;
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
    nonce_hex: String,
    ciphertext_hex: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum LocalSecretFile {
    Envelope(LocalSecretEnvelope),
    LegacyPlaintext(BTreeMap<String, String>),
}

fn read_local_secret_file(content: &str) -> CoreResult<BTreeMap<String, String>> {
    if content.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    match serde_json::from_str::<LocalSecretFile>(content)? {
        LocalSecretFile::Envelope(envelope) => decrypt_local_secret_values(&envelope),
        LocalSecretFile::LegacyPlaintext(values) => Ok(values),
    }
}

fn encrypt_local_secret_values(
    values: &BTreeMap<String, String>,
) -> CoreResult<LocalSecretEnvelope> {
    let key_bytes = derive_local_secret_key();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key_bytes));
    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes).map_err(|error| CoreError::External {
        service: "local_secret_store".to_owned(),
        message: format!("failed to generate secret nonce: {error}"),
    })?;
    let plaintext = serde_json::to_vec(values)?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
        .map_err(local_secret_crypto_error)?;
    Ok(LocalSecretEnvelope {
        version: 2,
        cipher: "chacha20poly1305".to_owned(),
        kdf: local_secret_kdf_label().to_owned(),
        nonce_hex: encode_hex(&nonce_bytes),
        ciphertext_hex: encode_hex(&ciphertext),
    })
}

fn decrypt_local_secret_values(
    envelope: &LocalSecretEnvelope,
) -> CoreResult<BTreeMap<String, String>> {
    if envelope.version != 2 {
        return Err(CoreError::validation(format!(
            "unsupported local secret store version {}",
            envelope.version
        )));
    }
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
    let key_bytes = derive_local_secret_key();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key_bytes));
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(local_secret_crypto_error)?;
    serde_json::from_slice(&plaintext).map_err(CoreError::from)
}

fn derive_local_secret_key() -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"ariadne-local-secret-store-v2");
    if let Ok(secret) = std::env::var("ARIADNE_SECRET_MASTER_KEY") {
        if !secret.trim().is_empty() {
            hasher.update(b"\0env-master-key\0");
            hasher.update(secret.as_bytes());
            return digest_to_key(hasher.finalize());
        }
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
    digest_to_key(hasher.finalize())
}

fn digest_to_key(digest: impl AsRef<[u8]>) -> [u8; 32] {
    let mut key = [0u8; 32];
    key.copy_from_slice(digest.as_ref());
    key
}

fn local_secret_kdf_label() -> &'static str {
    match std::env::var("ARIADNE_SECRET_MASTER_KEY") {
        Ok(value) if !value.trim().is_empty() => "env_master_key_sha256",
        _ => "machine_bound_sha256",
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
    if value.len() % 2 != 0 {
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
    fn local_file_secret_store_persists_between_instances() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("secrets.json");
        let store = LocalFileSecretStore::new(&path);
        store
            .set_secret("openai-main", SecretValue::new("sk-secret"))
            .unwrap();

        let reloaded = LocalFileSecretStore::new(&path);
        let secret = reloaded.get_secret("openai-main").unwrap().unwrap();
        assert_eq!(secret.expose_secret(), "sk-secret");

        let file = std::fs::read_to_string(&path).unwrap();
        assert!(file.contains("chacha20poly1305"));
        assert!(!file.contains("sk-secret"));
    }

    #[test]
    fn local_file_secret_store_reads_legacy_plaintext_and_rewrites_encrypted() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("secrets.json");
        std::fs::write(&path, r#"{"legacy":"old-secret"}"#).unwrap();

        let store = LocalFileSecretStore::new(&path);
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
