use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use crate::core::{CoreError, CoreResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretRef {
    pub key_id: String,
}

impl SecretRef {
    pub fn new(key_id: impl Into<String>) -> Self {
        Self {
            key_id: key_id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

pub trait SecretStore: Send + Sync {
    fn set_secret(&self, key_id: &str, value: SecretValue) -> CoreResult<()>;
    fn get_secret(&self, key_id: &str) -> CoreResult<Option<SecretValue>>;
    fn delete_secret(&self, key_id: &str) -> CoreResult<()>;
}

#[derive(Debug, Clone, Default)]
pub struct MemorySecretStore {
    values: Arc<RwLock<BTreeMap<String, SecretValue>>>,
}

impl SecretStore for MemorySecretStore {
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

    fn get_secret(&self, key_id: &str) -> CoreResult<Option<SecretValue>> {
        let values = self
            .values
            .read()
            .map_err(|_| CoreError::validation("secret store lock poisoned"))?;
        Ok(values.get(key_id).cloned())
    }

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
#[derive(Debug, Clone)]
pub struct SystemKeychainSecretStore {
    service: String,
}

#[cfg(feature = "system-keychain")]
impl SystemKeychainSecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

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
    fn set_secret(&self, key_id: &str, value: SecretValue) -> CoreResult<()> {
        self.entry(key_id)?
            .set_password(value.expose_secret())
            .map_err(keyring_error)
    }

    fn get_secret(&self, key_id: &str) -> CoreResult<Option<SecretValue>> {
        match self.entry(key_id)?.get_password() {
            Ok(value) => Ok(Some(SecretValue::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keyring_error(error)),
        }
    }

    fn delete_secret(&self, key_id: &str) -> CoreResult<()> {
        match self.entry(key_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keyring_error(error)),
        }
    }
}

#[cfg(feature = "system-keychain")]
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
