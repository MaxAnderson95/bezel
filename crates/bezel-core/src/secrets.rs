//! Where Credential passwords live: never in `bezel.toml`, only in a store keyed by Credential
//! ID. The app's adapter is the system credential store; tests use `MemorySecrets`.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use crate::library::CredentialId;

/// A password store. Every call may block (the Keychain can show a prompt), so the Library runs
/// it off the UI thread.
pub trait Secrets: Send {
    /// `None` when nothing is stored for the Credential.
    fn read(&self, credential: &CredentialId) -> Result<Option<Secret>, SecretsError>;
    /// Replaces what the Credential had; a failed write leaves it as it was.
    fn write(&self, credential: &CredentialId, secret: &Secret) -> Result<(), SecretsError>;
    /// Nothing stored is not an error.
    fn delete(&self, credential: &CredentialId) -> Result<(), SecretsError>;
}

/// A password that never prints.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl From<String> for Secret {
    fn from(secret: String) -> Self {
        Self(secret)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(…)")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SecretsError(pub String);

/// Passwords held in memory and shared between clones, for tests.
#[derive(Clone, Default)]
pub struct MemorySecrets(Arc<Mutex<HashMap<CredentialId, Secret>>>);

impl MemorySecrets {
    pub fn get(&self, credential: &CredentialId) -> Option<Secret> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(credential)
            .cloned()
    }
}

impl Secrets for MemorySecrets {
    fn read(&self, credential: &CredentialId) -> Result<Option<Secret>, SecretsError> {
        Ok(self.get(credential))
    }

    fn write(&self, credential: &CredentialId, secret: &Secret) -> Result<(), SecretsError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(credential.clone(), secret.clone());
        Ok(())
    }

    fn delete(&self, credential: &CredentialId) -> Result<(), SecretsError> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).remove(credential);
        Ok(())
    }
}
