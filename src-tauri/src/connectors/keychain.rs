//! Per-connector secret storage. Real impl backs onto the OS Keychain via `keyring`.
use crate::types::{AppError, AppResult};
use sha2::{Digest, Sha256};
use std::path::Path;
use zeroize::Zeroizing;

pub trait ConnectorSecrets: Send + Sync {
    fn read(&self, id: &str) -> AppResult<Zeroizing<String>>;
    fn write(&self, id: &str, secret: &str) -> AppResult<()>;
    fn remove(&self, id: &str) -> AppResult<()>;
}

fn service_name(id: &str) -> String {
    format!("forma-connector-google-{}", id)
}

fn error() -> AppError {
    AppError::new(
        "connector_credential",
        "The OS credential store is unavailable or access was denied.",
    )
}

pub struct OsSecrets {
    namespace: Option<String>,
}

impl OsSecrets {
    pub fn new(test_directory: Option<&Path>) -> Self {
        // Same directory-derived QA identity as core's OsCredentials. No fallback
        // to the production service when a QA namespace has no credential.
        let namespace = test_directory.map(|path| {
            format!(
                "dev.forma.connectors.qa.{:x}",
                Sha256::digest(path.as_os_str().to_string_lossy().as_bytes())
            )
        });
        Self { namespace }
    }
    fn service(&self, id: &str) -> String {
        match &self.namespace {
            Some(namespace) => format!("{namespace}.{id}"),
            None => service_name(id),
        }
    }
}

impl ConnectorSecrets for OsSecrets {
    fn read(&self, id: &str) -> AppResult<Zeroizing<String>> {
        let entry = keyring::Entry::new(&self.service(id), "tokens").map_err(|_| error())?;
        entry
            .get_password()
            .map(Zeroizing::new)
            .map_err(|_| error())
    }
    fn write(&self, id: &str, secret: &str) -> AppResult<()> {
        let entry = keyring::Entry::new(&self.service(id), "tokens").map_err(|_| error())?;
        entry.set_password(secret).map_err(|_| error())
    }
    fn remove(&self, id: &str) -> AppResult<()> {
        let entry = keyring::Entry::new(&self.service(id), "tokens").map_err(|_| error())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(error()),
        }
    }
}

/// Composio project API key: same OS-keychain custody as provider keys. The
/// value is never persisted, logged, or exposed past the native host.
pub trait ComposioKeys: Send + Sync {
    fn read(&self) -> AppResult<Zeroizing<String>>;
    fn has(&self) -> bool;
    fn write(&self, key: &str) -> AppResult<()>;
    fn remove(&self) -> AppResult<()>;
}

pub struct ComposioKeyStore {
    namespace: Option<String>,
}

impl ComposioKeyStore {
    const SERVICE: &'static str = "dev.forma.composio.v1";
    pub fn new(test_directory: Option<&std::path::Path>) -> Self {
        let namespace = test_directory.map(|path| {
            format!(
                "dev.forma.composio.qa.{:x}",
                Sha256::digest(path.as_os_str().to_string_lossy().as_bytes())
            )
        });
        Self { namespace }
    }
    fn service(&self) -> &str {
        self.namespace.as_deref().unwrap_or(Self::SERVICE)
    }
    fn entry(&self) -> AppResult<keyring::Entry> {
        keyring::Entry::new(self.service(), "api-key").map_err(|_| composio_key_error())
    }
}

fn composio_key_error() -> AppError {
    AppError::new(
        "composio_key_store",
        "The OS credential store is unavailable or access was denied. No key is saved in plaintext.",
    )
}

impl ComposioKeys for ComposioKeyStore {
    fn read(&self) -> AppResult<Zeroizing<String>> {
        let value = self
            .entry()?
            .get_password()
            .map_err(|_| composio_key_error())?;
        if value.is_empty() {
            return Err(AppError::new(
                "composio_not_configured",
                "No Composio API key is saved yet. Paste it in Settings, then try again.",
            ));
        }
        Ok(Zeroizing::new(value))
    }
    fn has(&self) -> bool {
        let Ok(entry) = self.entry() else {
            return false;
        };
        matches!(entry.get_password(), Ok(value) if !value.is_empty())
    }
    fn write(&self, key: &str) -> AppResult<()> {
        self.entry()?
            .set_password(key)
            .map_err(|_| composio_key_error())
    }
    fn remove(&self) -> AppResult<()> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(composio_key_error()),
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryKeys(pub Mutex<Option<String>>);

    impl ComposioKeys for MemoryKeys {
        fn read(&self) -> AppResult<Zeroizing<String>> {
            self.0
                .lock()
                .unwrap()
                .clone()
                .filter(|value| !value.is_empty())
                .map(Zeroizing::new)
                .ok_or_else(|| {
                    AppError::new(
                        "composio_not_configured",
                        "No Composio API key is saved yet.",
                    )
                })
        }
        fn has(&self) -> bool {
            self.0
                .lock()
                .unwrap()
                .as_deref()
                .is_some_and(|value| !value.is_empty())
        }
        fn write(&self, key: &str) -> AppResult<()> {
            *self.0.lock().unwrap() = Some(key.to_owned());
            Ok(())
        }
        fn remove(&self) -> AppResult<()> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    #[derive(Default)]
    pub struct MemorySecrets(pub Mutex<HashMap<String, String>>);

    impl ConnectorSecrets for MemorySecrets {
        fn read(&self, id: &str) -> AppResult<Zeroizing<String>> {
            self.0
                .lock()
                .unwrap()
                .get(id)
                .cloned()
                .map(Zeroizing::new)
                .ok_or_else(|| error())
        }
        fn write(&self, id: &str, secret: &str) -> AppResult<()> {
            self.0.lock().unwrap().insert(id.into(), secret.into());
            Ok(())
        }
        fn remove(&self, id: &str) -> AppResult<()> {
            self.0.lock().unwrap().remove(id);
            Ok(())
        }
    }

    #[test]
    fn service_name_is_per_connector() {
        assert_eq!(service_name("abc"), "forma-connector-google-abc");
        assert_ne!(service_name("a"), service_name("b"));
    }

    #[test]
    fn qa_namespaces_never_match_production_or_each_other() {
        let production = OsSecrets::new(None);
        let qa_a = OsSecrets::new(Some(Path::new("/tmp/forma-qa-a")));
        let qa_b = OsSecrets::new(Some(Path::new("/tmp/forma-qa-b")));
        assert_ne!(production.service("same-id"), qa_a.service("same-id"));
        assert_ne!(qa_a.service("same-id"), qa_b.service("same-id"));
        assert_eq!(
            qa_a.service("same-id"),
            OsSecrets::new(Some(Path::new("/tmp/forma-qa-a"))).service("same-id")
        );
        assert!(qa_a
            .service("same-id")
            .starts_with("dev.forma.connectors.qa."));
        assert_eq!(
            production.service("same-id"),
            "forma-connector-google-same-id"
        );
    }

    #[test]
    fn memory_impl_supports_round_trip_and_missing() {
        let secrets = MemorySecrets::default();
        assert!(secrets.read("nope").is_err());
        secrets.write("id-1", "payload").unwrap();
        assert_eq!(secrets.read("id-1").unwrap().as_str(), "payload");
        secrets.remove("id-1").unwrap();
        assert!(secrets.read("id-1").is_err());
        // Removing an already-absent entry is a no-op.
        secrets.remove("id-1").unwrap();
    }

    #[test]
    fn composio_key_namespaces_never_match_production_or_each_other() {
        let production = ComposioKeyStore::new(None);
        let qa_a = ComposioKeyStore::new(Some(Path::new("/tmp/forma-qa-a")));
        let qa_b = ComposioKeyStore::new(Some(Path::new("/tmp/forma-qa-b")));
        assert_eq!(production.service(), "dev.forma.composio.v1");
        assert_ne!(production.service(), qa_a.service());
        assert_ne!(qa_a.service(), qa_b.service());
        assert!(qa_a.service().starts_with("dev.forma.composio.qa."));
    }

    #[test]
    fn memory_keys_round_trip_and_missing() {
        let keys = MemoryKeys::default();
        assert!(!keys.has());
        assert_eq!(keys.read().unwrap_err().code, "composio_not_configured");
        keys.write("ak_synthetic").unwrap();
        assert!(keys.has());
        assert_eq!(keys.read().unwrap().as_str(), "ak_synthetic");
        keys.remove().unwrap();
        assert!(!keys.has());
        // Removing an already-absent entry is a no-op.
        keys.remove().unwrap();
    }
}
