//! Credential passwords in the system credential store (the Keychain on macOS) through
//! `rune::Credentials`. Headless runs (`--snap`) get Rune's in-memory store instead.

use bezel_core::library::CredentialId;
use bezel_core::secrets::{Secret, Secrets, SecretsError};
use rune::credentials::Credentials;

/// Rune keeps one item per service, so each Credential has a service of its own: this prefix,
/// a dot and the Credential ID, with the ID as the account too.
pub const SERVICE: &str = "io.github.maxanderson95.bezel";

pub struct Keychain(Credentials);

impl Keychain {
    pub fn system() -> Self {
        Self(Credentials::system())
    }
}

fn service(credential: &CredentialId) -> String {
    format!("{SERVICE}.{credential}")
}

/// Names the Keychain item, so a failure can be found and fixed in Keychain Access.
fn error(service: &str, e: impl std::fmt::Display) -> SecretsError {
    SecretsError(format!("{service}: {e}"))
}

impl Secrets for Keychain {
    fn read(&self, credential: &CredentialId) -> Result<Option<Secret>, SecretsError> {
        let service = service(credential);
        let Some(found) = self.0.read(&service).map_err(|e| error(&service, e))? else {
            return Ok(None);
        };
        String::from_utf8(found.secret().to_vec())
            .map(|password| Some(Secret::from(password)))
            .map_err(|_| error(&service, "the stored password is not text"))
    }

    fn write(&self, credential: &CredentialId, secret: &Secret) -> Result<(), SecretsError> {
        let service = service(credential);
        self.0
            .write(&service, credential.as_str(), secret.expose().as_bytes())
            .map_err(|e| error(&service, e))
    }

    fn delete(&self, credential: &CredentialId) -> Result<(), SecretsError> {
        let service = service(credential);
        self.0.delete(&service).map_err(|e| error(&service, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_credential_keeps_its_own_password() {
        let keychain = Keychain(Credentials::memory());
        let (admin, local) = (CredentialId::from("c_admin"), CredentialId::from("c_local"));
        keychain.write(&admin, &Secret::from("one".to_owned())).unwrap();
        keychain.write(&local, &Secret::from("two".to_owned())).unwrap();
        assert_eq!(keychain.read(&admin).unwrap(), Some(Secret::from("one".to_owned())));
        assert_eq!(keychain.read(&local).unwrap(), Some(Secret::from("two".to_owned())));
        keychain.delete(&admin).unwrap();
        assert_eq!(keychain.read(&admin).unwrap(), None);
        assert_eq!(keychain.read(&local).unwrap(), Some(Secret::from("two".to_owned())));
    }
}
