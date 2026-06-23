use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use crate::core::{CoreError, CoreResult};

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
    fn default() -> Self {
        Self::new("literature-agent")
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
}
