use std::sync::Once;

use keyring::Entry;

static INIT_CREDENTIAL_STORE: Once = Once::new();

/// Service-scoped access to Koharu's platform-backed string secret storage.
#[derive(Debug, Clone)]
pub struct SecretStore {
    service: String,
}

impl SecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// Load a secret by key, returning `None` when no credential exists.
    pub fn get(&self, key: &str) -> anyhow::Result<Option<String>> {
        let entry = secret_entry(&self.service, key)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    /// Store a secret by key. Use `delete` to clear an existing credential.
    pub fn set(&self, key: &str, secret: &str) -> anyhow::Result<()> {
        secret_entry(&self.service, key)?.set_password(secret)?;
        Ok(())
    }

    /// Clear a secret by key. Missing credentials are treated as success.
    pub fn delete(&self, key: &str) -> anyhow::Result<()> {
        match secret_entry(&self.service, key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.into()),
        }
    }
}

fn secret_entry(service: &str, key: &str) -> anyhow::Result<Entry> {
    INIT_CREDENTIAL_STORE.call_once(crate::platform::configure);
    Ok(Entry::new(service, key)?)
}
