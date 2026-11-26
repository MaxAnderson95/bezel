//! The persistent model in `bezel.toml`: Connections, Groups, Credentials, Settings and Trusted
//! certificates. Passwords never enter the file; they go to the `Secrets` store, keyed by
//! Credential ID. Tables, and keys inside known tables, that this version doesn't know are
//! carried through every save so a later Bezel's data and hand edits survive.

use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::Date;

use crate::address::{Address, DEFAULT_PORT};
use crate::keys::CommandKey;
use crate::secrets::{Secret, Secrets, SecretsError};
use crate::trust::{Fingerprint, TrustedCertificate};
use crate::usernames::Username;

const VERSION: u32 = 1;

macro_rules! id {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// A new random ID: the prefix, an underscore and six letters or digits.
            pub fn random() -> Self {
                Self(random_id($prefix))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(id: &str) -> Self {
                Self(id.into())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id!(
    /// Names a Connection for good; renaming never changes it.
    ConnectionId,
    "n"
);
id!(
    /// Names a Group for good; renaming never changes it.
    GroupId,
    "g"
);
id!(
    /// Names a Credential for good, and its password in the `Secrets` store.
    CredentialId,
    "c"
);

/// Random enough that two IDs made by any Bezel never meet; a collision is refused anyway.
fn random_id(prefix: &str) -> String {
    use std::hash::{BuildHasher as _, Hasher as _, RandomState};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    hasher.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    let mut n = hasher.finish();
    let mut id = format!("{prefix}_");
    for _ in 0..6 {
        id.push(char::from(DIGITS[(n % 36) as usize]));
        n /= 36;
    }
    id
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    pub id: CredentialId,
    pub name: String,
    /// As typed: `DOMAIN\user`, `user@domain` or `user`.
    pub username: String,
    #[serde(default, with = "toml_date::option", skip_serializing_if = "Option::is_none")]
    pub password_updated: Option<Date>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    pub id: ConnectionId,
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// `None` asks for a username and password each time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<CredentialId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<GroupId>,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

impl Connection {
    pub fn address(&self) -> Address {
        Address {
            host: self.host.clone(),
            port: self.port,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
    pub command_key: CommandKey,
    /// The default of the certificate dialog's "Remember this certificate".
    pub remember_certificates: bool,
}

/// One setting changed, leaving the others as they are on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Appearance(Appearance),
    CommandKey(CommandKey),
    RememberCertificates(bool),
}

impl Settings {
    pub fn with(mut self, setting: Setting) -> Self {
        match setting {
            Setting::Appearance(appearance) => self.appearance = appearance,
            Setting::CommandKey(command_key) => self.command_key = command_key,
            Setting::RememberCertificates(remember) => self.remember_certificates = remember,
        }
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub settings: Settings,
    /// In the order Max set.
    pub groups: Vec<Group>,
    pub credentials: Vec<Credential>,
    pub connections: Vec<Connection>,
    pub trusted_certificates: Vec<TrustedCertificate>,
}

/// One part of the sidebar: a Group, or the Connections without one (`group` is `None`).
#[derive(Debug, PartialEq, Eq)]
pub struct Section<'a> {
    pub group: Option<&'a Group>,
    pub connections: Vec<&'a Connection>,
}

impl Snapshot {
    pub fn trusted_certificate(&self, host: &str, port: u16) -> Option<&TrustedCertificate> {
        self.trusted_certificates.iter().find(|t| t.is_for(host, port))
    }

    pub fn connection(&self, id: &ConnectionId) -> Option<&Connection> {
        self.connections.iter().find(|c| &c.id == id)
    }

    pub fn group(&self, id: &GroupId) -> Option<&Group> {
        self.groups.iter().find(|g| &g.id == id)
    }

    pub fn credential(&self, id: &CredentialId) -> Option<&Credential> {
        self.credentials.iter().find(|c| &c.id == id)
    }

    /// The Connection's Credential, or `None` when it asks each time (a Credential missing
    /// from a hand-edited file asks too).
    pub fn credential_of(&self, connection: &Connection) -> Option<&Credential> {
        connection.credential.as_ref().and_then(|id| self.credential(id))
    }

    /// Every Group in order, each with its Connections alphabetically, then "Ungrouped" when
    /// any Connection has no Group (or one missing from a hand-edited file).
    pub fn sections(&self) -> Vec<Section<'_>> {
        let mut sections: Vec<Section<'_>> = self
            .groups
            .iter()
            .map(|group| Section {
                group: Some(group),
                connections: Vec::new(),
            })
            .collect();
        let mut ungrouped = Vec::new();
        for connection in &self.connections {
            let section = connection
                .group
                .as_ref()
                .and_then(|id| sections.iter_mut().find(|s| s.group.is_some_and(|g| &g.id == id)));
            match section {
                Some(section) => section.connections.push(connection),
                None => ungrouped.push(connection),
            }
        }
        if !ungrouped.is_empty() {
            sections.push(Section {
                group: None,
                connections: ungrouped,
            });
        }
        for section in &mut sections {
            section.connections.sort_by_key(|c| alphabetical(c));
        }
        sections
    }

    /// The Connections that sign in with the Credential, alphabetically.
    pub fn used_by(&self, credential: &CredentialId) -> Vec<&Connection> {
        let mut connections: Vec<&Connection> = self
            .connections
            .iter()
            .filter(|c| c.credential.as_ref() == Some(credential))
            .collect();
        connections.sort_by_key(|c| alphabetical(c));
        connections
    }
}

fn alphabetical(connection: &Connection) -> (String, String) {
    (connection.name.to_lowercase(), connection.host.to_lowercase())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// Stores the certificate in place of `replacing`, the fingerprint of the Trusted
    /// certificate the person saw for that host and port (`None` when none was trusted). If the
    /// file holds a different one by then, the change is refused.
    RememberCertificate {
        certificate: TrustedCertificate,
        replacing: Option<Fingerprint>,
    },
    /// Forgets the Trusted certificate for that host and port, refused if the file holds one
    /// other than `sha256`, the one the person saw.
    ForgetCertificate {
        host: String,
        port: u16,
        sha256: Fingerprint,
    },
    /// Its Credential and Group must exist.
    AddConnection(Connection),
    /// Changes the fields where `edited` differs from `original`, the Connection as the person
    /// saw it, and keeps the others as they are on disk, so an edit made meanwhile elsewhere
    /// survives. The Connection must still exist.
    EditConnection {
        original: Connection,
        edited: Connection,
    },
    DeleteConnection(ConnectionId),
    MoveConnection {
        connection: ConnectionId,
        group: Option<GroupId>,
    },
    /// Gives a Connection that asks each time the Credential just saved while signing in to
    /// `host`:`port`. Nothing changes when the Connection has a Credential by then or reaches
    /// another host or port: the sign-in was for that endpoint, and a later choice wins.
    AssignCredential {
        connection: ConnectionId,
        credential: CredentialId,
        host: String,
        port: u16,
    },
    AddGroup(Group),
    /// Refused when the Group's name on disk is no longer `from`, the one the person saw.
    RenameGroup {
        group: GroupId,
        from: String,
        name: String,
    },
    /// Its Connections become ungrouped.
    DeleteGroup(GroupId),
    /// Places the Group just before `before`, or last when `None`.
    MoveGroup {
        group: GroupId,
        before: Option<GroupId>,
    },
    /// Stores the password, then the Credential.
    AddCredential {
        credential: Credential,
        password: Secret,
    },
    /// Changes the name and username where `edited` differs from `original`, the Credential as
    /// the person saw it, and keeps the rest as it is on disk. A new password dates the
    /// Credential's `password_updated` to `today`, and every Connection using the Credential
    /// signs in with it from then on; it is refused if the username on disk is no longer the
    /// one the person saw, as the password was typed for that account.
    EditCredential {
        original: Credential,
        edited: Credential,
        password: Option<Secret>,
        today: Date,
    },
    /// Its Connections switch to asking each time, and its password is removed.
    DeleteCredential(CredentialId),
    Setting(Setting),
}

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("{} could not be read: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{} has an error{}: {message}", path.display(), line.map(|l| format!(" on line {l}")).unwrap_or_default())]
    Parse {
        path: PathBuf,
        line: Option<usize>,
        message: String,
    },
    #[error("{} is version {found}, newer than this Bezel understands", path.display())]
    Version { path: PathBuf, found: u32 },
    #[error("{} could not be saved: {source}", path.display())]
    Write { path: PathBuf, source: io::Error },
    #[error("A different certificate for {host}:{port} was remembered while this one was being checked")]
    Conflict { host: String, port: u16 },
    /// The change was made against data that has since changed on disk.
    #[error("{0}")]
    Stale(String),
    #[error("{0}")]
    Invalid(String),
    #[error("The Keychain refused: {0}")]
    Secrets(#[from] SecretsError),
    /// The change failed, and putting the Keychain back as it was failed too, leaving the
    /// Credential's item out of step with the file.
    #[error("{error}. The Keychain item for credential {credential} could not be put back as it was: {restore}")]
    Restore {
        error: Box<LibraryError>,
        credential: CredentialId,
        restore: SecretsError,
    },
}

pub struct Library {
    path: PathBuf,
    file: File,
    secrets: Box<dyn Secrets>,
}

impl Library {
    /// A missing file is an empty Library; the file is created by the first change. A file
    /// that fails to parse is an error, and no Library exists to overwrite it.
    pub fn open(path: impl Into<PathBuf>, secrets: impl Secrets + 'static) -> Result<Self, LibraryError> {
        let path = path.into();
        let file = read(&path)?;
        Ok(Self {
            path,
            file,
            secrets: Box::new(secrets),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn snapshot(&self) -> Snapshot {
        let file = &self.file;
        Snapshot {
            settings: file.settings.item,
            groups: items(&file.groups),
            credentials: items(&file.credentials),
            connections: items(&file.connections),
            trusted_certificates: items(&file.trusted_certificates),
        }
    }

    /// The Credential's password; `None` when none is stored. May block on a Keychain prompt.
    pub fn password(&self, credential: &CredentialId) -> Result<Option<Secret>, LibraryError> {
        Ok(self.secrets.read(credential)?)
    }

    /// Applies the change to the file as it is on disk now and saves it. A hand edit or another
    /// Bezel may have changed the file since it was opened, so the change never replaces it
    /// with an older copy: a change that no longer fits what is on disk is refused (`Stale` or
    /// `Conflict`) and the Library takes the file as it is, so the next decision is made on
    /// current data. A file that no longer parses is refused rather than overwritten.
    ///
    /// On any error the file and the Keychain are left as they were and the Library takes the
    /// file as it is on disk. A change to a password reaches the Keychain before the file is
    /// saved; if the save then fails, the previous password is put back, and a failure to put
    /// it back is reported with the Credential's ID (`Restore`).
    pub fn apply(&mut self, change: Change) -> Result<(), LibraryError> {
        let _lock = lock(&self.path)?;
        let disk = read(&self.path)?;
        let mut file = disk.clone();
        match file.change(change).and_then(|password| self.save(&file, password)) {
            Ok(()) => {
                self.file = file;
                Ok(())
            }
            Err(e) => {
                self.file = disk;
                Err(e)
            }
        }
    }

    /// Sets the Credential's password as the change asks, then saves the file, putting the
    /// previous password back if the save fails.
    fn save(&self, file: &File, password: PasswordChange) -> Result<(), LibraryError> {
        let Some((id, password)) = password else {
            return save(&self.path, file);
        };
        let previous = self.secrets.read(&id)?;
        set(&*self.secrets, &id, password.as_ref())?;
        save(&self.path, file).map_err(|error| match set(&*self.secrets, &id, previous.as_ref()) {
            Ok(()) => error,
            Err(restore) => LibraryError::Restore {
                error: Box::new(error),
                credential: id,
                restore,
            },
        })
    }
}

/// Stores the password for the Credential, or removes it for `None`.
fn set(secrets: &dyn Secrets, id: &CredentialId, password: Option<&Secret>) -> Result<(), SecretsError> {
    match password {
        Some(password) => secrets.write(id, password),
        None => secrets.delete(id),
    }
}

fn items<T: Clone>(entries: &[Entry<T>]) -> Vec<T> {
    entries.iter().map(|e| e.item.clone()).collect()
}

/// An item as the file holds it: the fields this Bezel knows, and any others, kept as they are.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Entry<T> {
    #[serde(flatten)]
    item: T,
    #[serde(flatten)]
    extra: toml::Table,
}

impl<T> Entry<T> {
    fn new(item: T) -> Self {
        Self {
            item,
            extra: toml::Table::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct File {
    version: u32,
    #[serde(default)]
    settings: Entry<Settings>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    groups: Vec<Entry<Group>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    credentials: Vec<Entry<Credential>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    connections: Vec<Entry<Connection>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    trusted_certificates: Vec<Entry<TrustedCertificate>>,
    #[serde(flatten)]
    rest: toml::Table,
}

impl Default for File {
    fn default() -> Self {
        Self {
            version: VERSION,
            settings: Entry::default(),
            groups: Vec::new(),
            credentials: Vec::new(),
            connections: Vec::new(),
            trusted_certificates: Vec::new(),
            rest: toml::Table::new(),
        }
    }
}

/// `File` without the unknown keys, read only to find where a parse error is.
#[derive(Deserialize)]
#[expect(dead_code, reason = "read for its errors")]
struct Precise {
    version: u32,
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    groups: Vec<Group>,
    #[serde(default)]
    credentials: Vec<Credential>,
    #[serde(default)]
    connections: Vec<Connection>,
    #[serde(default)]
    trusted_certificates: Vec<TrustedCertificate>,
}

/// A Credential whose password a change stores (`Some`) or removes (`None`).
type PasswordChange = Option<(CredentialId, Option<Secret>)>;

impl File {
    /// Applies the change after checking it against what this file holds. A refusal may come
    /// part way through, so the caller discards a file this refused.
    fn change(&mut self, change: Change) -> Result<PasswordChange, LibraryError> {
        match change {
            Change::RememberCertificate { certificate, replacing } => {
                let on_disk = self
                    .trusted_certificates
                    .iter()
                    .find(|t| t.item.is_for(&certificate.host, certificate.port))
                    .map(|t| t.item.sha256);
                if on_disk != replacing && on_disk != Some(certificate.sha256) {
                    return Err(LibraryError::Conflict {
                        host: certificate.host,
                        port: certificate.port,
                    });
                }
                let existing = self
                    .trusted_certificates
                    .iter_mut()
                    .find(|t| t.item.is_for(&certificate.host, certificate.port));
                match existing {
                    Some(entry) => entry.item = certificate,
                    None => self.trusted_certificates.push(Entry::new(certificate)),
                }
            }
            Change::ForgetCertificate { host, port, sha256 } => {
                let on_disk = self.trusted_certificates.iter().find(|t| t.item.is_for(&host, port));
                if on_disk.is_some_and(|t| t.item.sha256 != sha256) {
                    return Err(LibraryError::Stale(format!(
                        "The certificate for {host}:{port} was replaced since the list was shown, so Bezel kept it"
                    )));
                }
                self.trusted_certificates.retain(|t| !t.item.is_for(&host, port));
            }
            Change::AddConnection(connection) => {
                let connection = self.checked(connection)?;
                if self.connections.iter().any(|c| c.item.id == connection.id) {
                    return Err(taken(&connection.name));
                }
                self.connections.push(Entry::new(connection));
            }
            Change::EditConnection { original, edited } => {
                let index = self
                    .connections
                    .iter()
                    .position(|c| c.item.id == original.id)
                    .ok_or_else(|| gone(&original.name))?;
                let disk = &self.connections[index].item;
                let field = Field::new(&original.name);
                let merged = Connection {
                    id: original.id.clone(),
                    name: field.merge("name", &disk.name, &original.name, edited.name)?,
                    host: field.merge("host", &disk.host, &original.host, edited.host)?,
                    port: field.merge("port", &disk.port, &original.port, edited.port)?,
                    credential: field.merge("credential", &disk.credential, &original.credential, edited.credential)?,
                    group: field.merge("group", &disk.group, &original.group, edited.group)?,
                };
                self.connections[index].item = self.checked(merged)?;
            }
            Change::DeleteConnection(id) => self.connections.retain(|c| c.item.id != id),
            Change::MoveConnection { connection, group } => {
                if let Some(group) = &group {
                    self.group(group)?;
                }
                let entry = self
                    .connections
                    .iter_mut()
                    .find(|c| c.item.id == connection)
                    .ok_or_else(|| gone("The connection"))?;
                entry.item.group = group;
            }
            Change::AssignCredential {
                connection,
                credential,
                host,
                port,
            } => {
                self.credential(&credential)?;
                let entry = self.connections.iter_mut().find(|c| c.item.id == connection);
                if let Some(entry) = entry
                    && entry.item.credential.is_none()
                    && entry.item.host.eq_ignore_ascii_case(&host)
                    && entry.item.port == port
                {
                    entry.item.credential = Some(credential);
                }
            }
            Change::AddGroup(group) => {
                let group = Group {
                    name: required(&group.name, "Enter a name for the group")?,
                    ..group
                };
                if self.groups.iter().any(|g| g.item.id == group.id) {
                    return Err(taken(&group.name));
                }
                self.groups.push(Entry::new(group));
            }
            Change::RenameGroup { group, from, name } => {
                let name = required(&name, "Enter a name for the group")?;
                let index = self.group(&group)?;
                let item = &mut self.groups[index].item;
                item.name = Field::new(&from).merge("name", &item.name, &from, name)?;
            }
            Change::DeleteGroup(id) => {
                self.groups.retain(|g| g.item.id != id);
                for connection in &mut self.connections {
                    if connection.item.group.as_ref() == Some(&id) {
                        connection.item.group = None;
                    }
                }
            }
            Change::MoveGroup { group, before } => {
                let from = self.group(&group)?;
                if let Some(before) = &before {
                    self.group(before)?;
                }
                if before.as_ref() == Some(&group) {
                    return Ok(None);
                }
                let entry = self.groups.remove(from);
                let to = match &before {
                    Some(before) => self.group(before)?,
                    None => self.groups.len(),
                };
                self.groups.insert(to, entry);
            }
            Change::AddCredential { credential, password } => {
                let credential = Credential {
                    name: required(&credential.name, "Enter a name for the credential")?,
                    username: username(&credential.username)?,
                    ..credential
                };
                if self.credentials.iter().any(|c| c.item.id == credential.id) {
                    return Err(taken(&credential.name));
                }
                let id = credential.id.clone();
                self.credentials.push(Entry::new(credential));
                return Ok(Some((id, Some(password))));
            }
            Change::EditCredential {
                original,
                edited,
                password,
                today,
            } => {
                let index = self.credential(&original.id)?;
                let disk = &self.credentials[index].item;
                if password.is_some() && disk.username != original.username {
                    return Err(LibraryError::Stale(format!(
                        "The username of {} changed to {} while the new password was typed, so the password was not saved",
                        disk.name, disk.username
                    )));
                }
                let field = Field::new(&original.name);
                let name = required(
                    &field.merge("name", &disk.name, &original.name, edited.name)?,
                    "Enter a name for the credential",
                )?;
                let new_username =
                    username(&field.merge("username", &disk.username, &original.username, edited.username)?)?;
                let item = &mut self.credentials[index].item;
                item.name = name;
                item.username = new_username;
                if let Some(password) = password {
                    item.password_updated = Some(today);
                    return Ok(Some((original.id, Some(password))));
                }
            }
            Change::DeleteCredential(id) => {
                self.credentials.retain(|c| c.item.id != id);
                for connection in &mut self.connections {
                    if connection.item.credential.as_ref() == Some(&id) {
                        connection.item.credential = None;
                    }
                }
                return Ok(Some((id, None)));
            }
            Change::Setting(setting) => self.settings.item = self.settings.item.with(setting),
        }
        Ok(None)
    }

    /// The Connection with its text trimmed, once its fields are valid and the Credential and
    /// Group it names are still in the file.
    fn checked(&self, connection: Connection) -> Result<Connection, LibraryError> {
        let name = required(&connection.name, "Enter a name for the connection")?;
        let host = required(&connection.host, "Enter a host name")?;
        if host.contains(char::is_whitespace) {
            return Err(LibraryError::Invalid("A host name can't contain spaces".into()));
        }
        if connection.port == 0 {
            return Err(LibraryError::Invalid("\"0\" is not a port".into()));
        }
        if let Some(credential) = &connection.credential {
            self.credential(credential)
                .map_err(|_| LibraryError::Stale(format!("The credential chosen for {name} was deleted")))?;
        }
        if let Some(group) = &connection.group {
            self.group(group)
                .map_err(|_| LibraryError::Stale(format!("The group chosen for {name} was deleted")))?;
        }
        Ok(Connection {
            name,
            host,
            ..connection
        })
    }

    fn group(&self, id: &GroupId) -> Result<usize, LibraryError> {
        self.groups
            .iter()
            .position(|g| &g.item.id == id)
            .ok_or_else(|| gone("The group"))
    }

    fn credential(&self, id: &CredentialId) -> Result<usize, LibraryError> {
        self.credentials
            .iter()
            .position(|c| &c.item.id == id)
            .ok_or_else(|| gone("The credential"))
    }
}

/// Merges an edit made from `original`, the item as the person saw it, into the item on disk.
struct Field<'a> {
    /// The item's name as the person saw it, for messages.
    item: &'a str,
}

impl<'a> Field<'a> {
    fn new(item: &'a str) -> Self {
        Self { item }
    }

    /// The value on disk where the person left the field alone, else the edited value; refused
    /// when the field was also changed on disk to something else, so neither change is lost
    /// without the person seeing it.
    fn merge<T: Clone + PartialEq>(&self, what: &str, disk: &T, original: &T, edited: T) -> Result<T, LibraryError> {
        if edited == *original {
            Ok(disk.clone())
        } else if disk == original || *disk == edited {
            Ok(edited)
        } else {
            Err(LibraryError::Stale(format!(
                "The {what} of {} was changed elsewhere while it was being edited; check it and save again",
                self.item
            )))
        }
    }
}

fn required(value: &str, message: &str) -> Result<String, LibraryError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(LibraryError::Invalid(message.into()));
    }
    Ok(value.into())
}

fn username(value: &str) -> Result<String, LibraryError> {
    Username::parse(value).map_err(|e| LibraryError::Invalid(e.to_string()))?;
    Ok(value.trim().into())
}

fn gone(what: &str) -> LibraryError {
    LibraryError::Stale(format!("{what} was deleted while this change was being made"))
}

fn taken(name: &str) -> LibraryError {
    LibraryError::Stale(format!("{name} could not be added because its ID is in use; try again"))
}

fn read(path: &Path) -> Result<File, LibraryError> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(File::default()),
        Err(source) => {
            return Err(LibraryError::Read {
                path: path.into(),
                source,
            });
        }
    };
    let file: File = toml::from_str(&source).map_err(|e| {
        // Reading through `Entry` keeps only the span of the item at fault, so the same text is
        // read again without it to name the line of the key itself.
        let e = toml::from_str::<Precise>(&source).err().unwrap_or(e);
        LibraryError::Parse {
            line: e.span().map(|span| source[..span.start].matches('\n').count() + 1),
            message: e.message().to_owned(),
            path: path.into(),
        }
    })?;
    if file.version != VERSION {
        return Err(LibraryError::Version {
            path: path.into(),
            found: file.version,
        });
    }
    Ok(file)
}

fn save(path: &Path, file: &File) -> Result<(), LibraryError> {
    let error = |source| LibraryError::Write {
        path: path.into(),
        source,
    };
    let text = toml::to_string(file).map_err(|e| error(io::Error::other(e)))?;
    write_atomically(path, text.as_bytes()).map_err(error)
}

/// Holds an exclusive lock on a file beside the Library's, so two Bezels saving at once take
/// turns instead of each writing over the other's change. Released when dropped.
fn lock(path: &Path) -> Result<fs::File, LibraryError> {
    let error = |source| LibraryError::Write {
        path: path.into(),
        source,
    };
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    let dir = parent(path);
    fs::create_dir_all(dir).map_err(error)?;
    let file = fs::File::create(dir.join(name)).map_err(error)?;
    file.lock().map_err(error)?;
    Ok(file)
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Writes beside the target and renames over it, so a crash leaves the old file or the new
/// one, never half of either.
fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    let dir = parent(path);
    fs::create_dir_all(dir)?;
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    let temp = dir.join(name);
    let written = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

/// TOML local dates (`2027-03-14`) as `time::Date`.
pub(crate) mod toml_date {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use time::Date;
    use toml::value::Datetime;

    pub fn serialize<S: Serializer>(date: &Date, serializer: S) -> Result<S::Ok, S::Error> {
        Datetime {
            date: Some(toml::value::Date {
                year: date.year() as u16,
                month: date.month().into(),
                day: date.day(),
            }),
            time: None,
            offset: None,
        }
        .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Date, D::Error> {
        let datetime = Datetime::deserialize(deserializer)?;
        let date = match datetime {
            Datetime {
                date: Some(date),
                time: None,
                offset: None,
            } => date,
            _ => return Err(serde::de::Error::custom("expected a date such as 2027-03-14")),
        };
        time::Month::try_from(date.month)
            .and_then(|month| Date::from_calendar_date(date.year.into(), month, date.day))
            .map_err(serde::de::Error::custom)
    }

    /// For an optional field that is left out when `None` (`skip_serializing_if`).
    pub mod option {
        use super::*;

        pub fn serialize<S: Serializer>(date: &Option<Date>, serializer: S) -> Result<S::Ok, S::Error> {
            match date {
                Some(date) => super::serialize(date, serializer),
                None => serializer.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Date>, D::Error> {
            super::deserialize(deserializer).map(Some)
        }
    }
}

#[cfg(test)]
mod tests {
    use time::macros::date;

    use super::*;
    use crate::secrets::MemorySecrets;
    use crate::trust::{Certificate, Decision, Question, SessionTrust};

    fn trusted(host: &str, fill: u8) -> TrustedCertificate {
        TrustedCertificate {
            host: host.into(),
            port: 3389,
            sha256: Fingerprint::of(&[fill]),
            subject: host.into(),
            issuer: host.into(),
            not_after: date!(2027 - 03 - 14),
            remembered: date!(2026 - 10 - 03),
        }
    }

    fn certificate(fill: u8) -> Certificate {
        Certificate {
            sha256: Fingerprint::of(&[fill]),
            subject: "fs01.corp.lan".into(),
            issuer: "fs01.corp.lan".into(),
            self_signed: true,
            not_before: date!(2026 - 03 - 14),
            not_after: date!(2027 - 03 - 14),
        }
    }

    fn remember(certificate: TrustedCertificate) -> Change {
        Change::RememberCertificate {
            certificate,
            replacing: None,
        }
    }

    fn group(id: &str, name: &str) -> Group {
        Group {
            id: id.into(),
            name: name.into(),
        }
    }

    fn credential(id: &str, name: &str) -> Credential {
        Credential {
            id: id.into(),
            name: name.into(),
            username: r"CORP\admin.max".into(),
            password_updated: Some(date!(2026 - 08 - 23)),
        }
    }

    fn connection(id: &str, name: &str, credential: Option<&str>, group: Option<&str>) -> Connection {
        Connection {
            id: id.into(),
            name: name.into(),
            host: format!("{name}.corp.lan"),
            port: 3389,
            credential: credential.map(Into::into),
            group: group.map(Into::into),
        }
    }

    fn admin() -> Credential {
        credential("c_admin", "Domain admin")
    }

    /// The password typed for the Credential as the person saw it.
    fn new_password(original: Credential, password: &str) -> Change {
        Change::EditCredential {
            edited: original.clone(),
            original,
            password: Some(Secret::from(password.to_owned())),
            today: date!(2026 - 11 - 25),
        }
    }

    /// Makes every save fail by putting a folder where Bezel writes its temporary file.
    #[must_use]
    fn block_saves(path: &Path) -> PathBuf {
        let blocker = path.with_file_name(format!("bezel.toml.{}.tmp", std::process::id()));
        fs::create_dir_all(&blocker).unwrap();
        blocker
    }

    /// `MemorySecrets` that refuses the operations named.
    #[derive(Clone, Default)]
    struct Refusing {
        inner: MemorySecrets,
        refused: std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>,
    }

    impl Refusing {
        fn refuse(&self, operation: &'static str) {
            self.refused.lock().unwrap().push(operation);
        }

        fn check(&self, operation: &str) -> Result<(), SecretsError> {
            match self.refused.lock().unwrap().contains(&operation) {
                true => Err(SecretsError(format!("{operation} denied"))),
                false => Ok(()),
            }
        }
    }

    impl Secrets for Refusing {
        fn read(&self, credential: &CredentialId) -> Result<Option<Secret>, SecretsError> {
            self.check("read")?;
            self.inner.read(credential)
        }

        fn write(&self, credential: &CredentialId, secret: &Secret) -> Result<(), SecretsError> {
            self.check("write")?;
            self.inner.write(credential, secret)
        }

        fn delete(&self, credential: &CredentialId) -> Result<(), SecretsError> {
            self.check("delete")?;
            self.inner.delete(credential)
        }
    }

    fn add_credential(id: &str, name: &str, password: &str) -> Change {
        Change::AddCredential {
            credential: credential(id, name),
            password: Secret::from(password.to_owned()),
        }
    }

    fn temp_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Bezel").join("bezel.toml");
        (dir, path)
    }

    fn open(path: &Path) -> Library {
        Library::open(path, MemorySecrets::default()).unwrap()
    }

    /// Two Groups, a Credential and three Connections, one of them ungrouped.
    fn populated(path: &Path, secrets: &MemorySecrets) -> Library {
        let mut library = Library::open(path, secrets.clone()).unwrap();
        for change in [
            Change::AddGroup(group("g_infra", "Infrastructure")),
            Change::AddGroup(group("g_files", "File servers")),
            add_credential("c_admin", "Domain admin", "hunter2"),
            Change::AddConnection(connection("n_dc02", "dc02", Some("c_admin"), Some("g_infra"))),
            Change::AddConnection(connection("n_dc01", "DC01", Some("c_admin"), Some("g_infra"))),
            Change::AddConnection(connection("n_lab", "lab", None, None)),
        ] {
            library.apply(change).unwrap();
        }
        library
    }

    #[test]
    fn a_missing_file_is_an_empty_library_and_nothing_is_written() {
        let (_dir, path) = temp_path();
        let library = open(&path);
        assert_eq!(library.snapshot(), Snapshot::default());
        assert!(!path.exists());
    }

    #[test]
    fn remembered_certificates_round_trip() {
        let (_dir, path) = temp_path();
        let mut library = open(&path);
        library.apply(remember(trusted("fs01.corp.lan", 1))).unwrap();
        library.apply(remember(trusted("dc01.corp.lan", 2))).unwrap();
        let reopened = open(&path).snapshot();
        assert_eq!(reopened, library.snapshot());
        assert_eq!(
            reopened.trusted_certificate("FS01.corp.lan", 3389),
            Some(&trusted("fs01.corp.lan", 1))
        );
    }

    #[test]
    fn writes_the_documented_format() {
        let (_dir, path) = temp_path();
        let secrets = MemorySecrets::default();
        let mut library = populated(&path, &secrets);
        library.apply(remember(trusted("fs01.corp.lan", 1))).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("version = 1\n"), "{text}");
        for expected in [
            "[settings]\nappearance = \"system\"\ncommand_key = \"mac-shortcuts\"\nremember_certificates = false\n",
            "[[groups]]\nid = \"g_infra\"\nname = \"Infrastructure\"\n",
            "[[credentials]]\nid = \"c_admin\"\nname = \"Domain admin\"\nusername = 'CORP\\admin.max'\npassword_updated = 2026-08-23\n",
            "[[connections]]\nid = \"n_dc02\"\nname = \"dc02\"\nhost = \"dc02.corp.lan\"\nport = 3389\ncredential = \"c_admin\"\ngroup = \"g_infra\"\n",
            "[[trusted_certificates]]",
            "not_after = 2027-03-14\n",
        ] {
            assert!(text.contains(expected), "{expected:?} missing from\n{text}");
        }
        assert!(!text.contains("hunter2"), "{text}");
        assert_eq!(open(&path).snapshot(), library.snapshot());
    }

    #[test]
    fn a_setting_changes_alone_and_keeps_the_others_as_they_are_on_disk() {
        let (_dir, path) = temp_path();
        let mut library = open(&path);
        library
            .apply(Change::Setting(Setting::Appearance(Appearance::Dark)))
            .unwrap();
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("command_key = \"mac-shortcuts\"", "command_key = \"windows-key\"");
        fs::write(&path, text).unwrap();
        library
            .apply(Change::Setting(Setting::RememberCertificates(true)))
            .unwrap();
        let expected = Settings {
            appearance: Appearance::Dark,
            command_key: CommandKey::WindowsKey,
            remember_certificates: true,
        };
        assert_eq!(library.snapshot().settings, expected);
        assert_eq!(open(&path).snapshot().settings, expected);
    }

    #[test]
    fn reads_the_example_in_the_spec() {
        let (_dir, path) = temp_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, SPEC_EXAMPLE).unwrap();
        let snapshot = open(&path).snapshot();
        assert_eq!(snapshot.settings, Settings::default());
        assert_eq!(snapshot.groups, vec![group("g_7k2m", "Infrastructure")]);
        assert_eq!(snapshot.credentials[0].username, r"CORP\admin.max");
        assert_eq!(snapshot.credentials[0].password_updated, Some(date!(2026 - 08 - 23)));
        let dc01 = &snapshot.connections[0];
        assert_eq!(dc01.credential, Some("c_9x4p".into()));
        assert_eq!(dc01.address(), Address::parse("dc01.corp.lan").unwrap());
        assert_eq!(snapshot.trusted_certificates[0].host, "fs01.corp.lan");
    }

    const SPEC_EXAMPLE: &str = r#"version = 1

[settings]
appearance = "system"          # system | light | dark
command_key = "mac-shortcuts"  # mac-shortcuts | windows-key
remember_certificates = false

[[groups]]
id = "g_7k2m"
name = "Infrastructure"

[[credentials]]
id = "c_9x4p"
name = "Domain admin"
username = 'CORP\admin.max'
password_updated = 2026-08-23

[[connections]]
id = "n_3j8q"
name = "dc01"
host = "dc01.corp.lan"
port = 3389          # optional
credential = "c_9x4p" # optional; absent means ask each time
group = "g_7k2m"      # optional

[[trusted_certificates]]
host = "fs01.corp.lan"
port = 3389
sha256 = "7F3A91C20D5E88B41A6CE34792FD057BC8219E44A06DF1385B72CE0913B68AE5"
subject = "fs01.corp.lan"
issuer = "fs01.corp.lan"
not_after = 2027-03-14
remembered = 2026-10-03
"#;

    #[test]
    fn the_sidebar_lists_groups_in_order_connections_alphabetically_and_ungrouped_last() {
        let (_dir, path) = temp_path();
        let mut library = populated(&path, &MemorySecrets::default());
        library
            .apply(Change::AddConnection(connection("n_orphan", "orphan", None, None)))
            .unwrap();
        let snapshot = library.snapshot();
        let sections: Vec<(Option<&str>, Vec<&str>)> = snapshot
            .sections()
            .iter()
            .map(|s| {
                (
                    s.group.map(|g| g.name.as_str()),
                    s.connections.iter().map(|c| c.name.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            sections,
            vec![
                (Some("Infrastructure"), vec!["DC01", "dc02"]),
                (Some("File servers"), vec![]),
                (None, vec!["lab", "orphan"]),
            ]
        );
    }

    #[test]
    fn a_connection_whose_group_is_missing_shows_as_ungrouped() {
        let mut snapshot = Snapshot::default();
        snapshot
            .connections
            .push(connection("n_dc01", "dc01", None, Some("g_gone")));
        let sections = snapshot.sections();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].group, None);
    }

    #[test]
    fn passwords_live_in_secrets_and_never_in_the_file() {
        let (_dir, path) = temp_path();
        let secrets = MemorySecrets::default();
        let library = populated(&path, &secrets);
        let id = CredentialId::from("c_admin");
        assert_eq!(secrets.get(&id), Some(Secret::from("hunter2".to_owned())));
        assert_eq!(library.password(&id).unwrap(), Some(Secret::from("hunter2".to_owned())));
        assert!(!fs::read_to_string(&path).unwrap().contains("hunter2"));
    }

    #[test]
    fn a_new_password_reaches_every_connection_using_the_credential() {
        let (_dir, path) = temp_path();
        let secrets = MemorySecrets::default();
        let mut library = populated(&path, &secrets);
        let id = CredentialId::from("c_admin");
        library.apply(new_password(admin(), "correct horse")).unwrap();
        let snapshot = open(&path).snapshot();
        let users: Vec<&str> = snapshot.used_by(&id).iter().map(|c| c.name.as_str()).collect();
        assert_eq!(users, ["DC01", "dc02"]);
        for user in snapshot.used_by(&id) {
            let credential = snapshot.credential_of(user).unwrap();
            assert_eq!(credential.password_updated, Some(date!(2026 - 11 - 25)));
            assert_eq!(
                library.password(&credential.id).unwrap(),
                Some(Secret::from("correct horse".to_owned()))
            );
        }
    }

    #[test]
    fn editing_a_credential_keeps_what_was_not_changed() {
        let (_dir, path) = temp_path();
        let mut library = populated(&path, &MemorySecrets::default());
        // Another Bezel changes the username after this one showed the Credential.
        open(&path)
            .apply(Change::EditCredential {
                original: admin(),
                edited: Credential {
                    username: r"CORP\max".into(),
                    ..admin()
                },
                password: None,
                today: date!(2026 - 11 - 25),
            })
            .unwrap();
        library
            .apply(Change::EditCredential {
                original: admin(),
                edited: Credential {
                    name: "  Admin ".into(),
                    ..admin()
                },
                password: None,
                today: date!(2026 - 11 - 25),
            })
            .unwrap();
        let credential = library.snapshot().credentials[0].clone();
        assert_eq!(credential.name, "Admin");
        assert_eq!(credential.username, r"CORP\max");
        assert_eq!(credential.password_updated, Some(date!(2026 - 08 - 23)));
    }

    #[test]
    fn a_new_password_is_refused_when_the_username_changed_meanwhile() {
        let (_dir, path) = temp_path();
        let secrets = MemorySecrets::default();
        let mut library = populated(&path, &secrets);
        let mut other = open(&path);
        let bob = Credential {
            username: r"CORP\bob".into(),
            ..admin()
        };
        other
            .apply(Change::EditCredential {
                original: admin(),
                edited: bob.clone(),
                password: None,
                today: date!(2026 - 11 - 25),
            })
            .unwrap();
        assert!(matches!(
            library.apply(new_password(admin(), "alice's")),
            Err(LibraryError::Stale(_))
        ));
        assert_eq!(secrets.get(&"c_admin".into()), Some(Secret::from("hunter2".to_owned())));
        assert_eq!(library.snapshot().credentials[0].username, bob.username);
    }

    #[test]
    fn deleting_a_credential_makes_its_connections_ask_and_removes_its_password() {
        let (_dir, path) = temp_path();
        let secrets = MemorySecrets::default();
        let mut library = populated(&path, &secrets);
        let id = CredentialId::from("c_admin");
        library.apply(Change::DeleteCredential(id.clone())).unwrap();
        let snapshot = open(&path).snapshot();
        assert!(snapshot.credentials.is_empty());
        assert!(snapshot.connections.iter().all(|c| c.credential.is_none()));
        assert_eq!(secrets.get(&id), None);
    }

    #[test]
    fn deleting_a_group_ungroups_its_connections() {
        let (_dir, path) = temp_path();
        let mut library = populated(&path, &MemorySecrets::default());
        library.apply(Change::DeleteGroup("g_infra".into())).unwrap();
        let snapshot = open(&path).snapshot();
        assert_eq!(snapshot.groups, vec![group("g_files", "File servers")]);
        assert!(snapshot.connections.iter().all(|c| c.group.is_none()));
    }

    #[test]
    fn groups_move_before_another_or_to_the_end() {
        let (_dir, path) = temp_path();
        let mut library = populated(&path, &MemorySecrets::default());
        library.apply(Change::AddGroup(group("g_lab", "Lab"))).unwrap();
        let order =
            |library: &Library| -> Vec<String> { library.snapshot().groups.into_iter().map(|g| g.name).collect() };
        library
            .apply(Change::MoveGroup {
                group: "g_lab".into(),
                before: Some("g_infra".into()),
            })
            .unwrap();
        assert_eq!(order(&library), ["Lab", "Infrastructure", "File servers"]);
        library
            .apply(Change::MoveGroup {
                group: "g_lab".into(),
                before: None,
            })
            .unwrap();
        assert_eq!(order(&library), ["Infrastructure", "File servers", "Lab"]);
        library
            .apply(Change::MoveGroup {
                group: "g_infra".into(),
                before: Some("g_lab".into()),
            })
            .unwrap();
        assert_eq!(order(&open(&path)), ["File servers", "Infrastructure", "Lab"]);
    }

    #[test]
    fn connections_move_between_groups_and_edits_replace_them() {
        let (_dir, path) = temp_path();
        let mut library = populated(&path, &MemorySecrets::default());
        library
            .apply(Change::MoveConnection {
                connection: "n_lab".into(),
                group: Some("g_files".into()),
            })
            .unwrap();
        let original = library.snapshot().connection(&"n_dc01".into()).unwrap().clone();
        let mut edited = connection("n_dc01", " dc01 ", None, None);
        edited.host = " dc01.corp.lan ".into();
        edited.port = 3390;
        library.apply(Change::EditConnection { original, edited }).unwrap();
        let snapshot = open(&path).snapshot();
        assert_eq!(
            snapshot.connection(&"n_lab".into()).unwrap().group,
            Some("g_files".into())
        );
        let dc01 = snapshot.connection(&"n_dc01".into()).unwrap();
        assert_eq!(
            (dc01.name.as_str(), dc01.host.as_str(), dc01.port),
            ("dc01", "dc01.corp.lan", 3390)
        );
        assert_eq!((&dc01.credential, &dc01.group), (&None, &None));
        library.apply(Change::DeleteConnection("n_dc01".into())).unwrap();
        assert!(open(&path).snapshot().connection(&"n_dc01".into()).is_none());
    }

    #[test]
    fn invalid_changes_are_refused() {
        let (_dir, path) = temp_path();
        let mut library = populated(&path, &MemorySecrets::default());
        let before = fs::read_to_string(&path).unwrap();
        let mut spaced = connection("n_new", "new", None, None);
        spaced.host = "dc 01".into();
        let mut no_port = connection("n_new", "new", None, None);
        no_port.port = 0;
        for change in [
            Change::AddConnection(connection("n_new", " ", None, None)),
            Change::AddConnection(spaced),
            Change::AddConnection(no_port),
            Change::AddGroup(group("g_new", "")),
            Change::RenameGroup {
                group: "g_infra".into(),
                from: "Infrastructure".into(),
                name: " ".into(),
            },
            Change::AddCredential {
                credential: Credential {
                    username: r"CORP\".into(),
                    ..credential("c_new", "New")
                },
                password: Secret::from(String::new()),
            },
        ] {
            assert!(
                matches!(library.apply(change.clone()), Err(LibraryError::Invalid(_))),
                "{change:?}"
            );
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn changes_made_against_deleted_items_are_refused_and_the_library_catches_up() {
        let (_dir, path) = temp_path();
        let mut stale = populated(&path, &MemorySecrets::default());
        let mut other = open(&path);
        other.apply(Change::DeleteConnection("n_dc01".into())).unwrap();
        other.apply(Change::DeleteCredential("c_admin".into())).unwrap();
        other.apply(Change::DeleteGroup("g_files".into())).unwrap();
        let on_disk = fs::read_to_string(&path).unwrap();
        for change in [
            Change::EditConnection {
                original: connection("n_dc01", "DC01", Some("c_admin"), Some("g_infra")),
                edited: connection("n_dc01", "dc01", None, None),
            },
            Change::AddConnection(connection("n_new", "new", Some("c_admin"), None)),
            Change::MoveConnection {
                connection: "n_lab".into(),
                group: Some("g_files".into()),
            },
            Change::RenameGroup {
                group: "g_files".into(),
                from: "File servers".into(),
                name: "Files".into(),
            },
            Change::MoveGroup {
                group: "g_infra".into(),
                before: Some("g_files".into()),
            },
            new_password(admin(), "new"),
        ] {
            assert!(
                matches!(stale.apply(change.clone()), Err(LibraryError::Stale(_))),
                "{change:?}"
            );
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), on_disk);
        assert_eq!(stale.snapshot(), other.snapshot());
    }

    #[test]
    fn edits_keep_fields_changed_on_disk_that_the_person_did_not_touch() {
        let (_dir, path) = temp_path();
        let mut library = populated(&path, &MemorySecrets::default());
        let original = library.snapshot().connection(&"n_dc01".into()).unwrap().clone();
        let mut other = open(&path);
        other
            .apply(Change::EditConnection {
                original: original.clone(),
                edited: Connection {
                    host: "dc01.new.lan".into(),
                    ..original.clone()
                },
            })
            .unwrap();
        library
            .apply(Change::EditConnection {
                original: original.clone(),
                edited: Connection {
                    name: "dc01 (primary)".into(),
                    ..original
                },
            })
            .unwrap();
        let dc01 = open(&path).snapshot().connection(&"n_dc01".into()).unwrap().clone();
        assert_eq!(
            (dc01.name.as_str(), dc01.host.as_str()),
            ("dc01 (primary)", "dc01.new.lan")
        );
    }

    #[test]
    fn edits_to_the_same_field_on_disk_are_refused() {
        let (_dir, path) = temp_path();
        let mut library = populated(&path, &MemorySecrets::default());
        let original = library.snapshot().connection(&"n_dc01".into()).unwrap().clone();
        let host = |host: &str| Change::EditConnection {
            original: original.clone(),
            edited: Connection {
                host: host.into(),
                ..original.clone()
            },
        };
        open(&path).apply(host("dc01-b.corp.lan")).unwrap();
        // The same value as the one on disk is no conflict.
        library.apply(host("dc01-b.corp.lan")).unwrap();
        assert!(matches!(
            library.apply(host("dc01-c.corp.lan")),
            Err(LibraryError::Stale(_))
        ));
        assert_eq!(
            library.snapshot().connection(&"n_dc01".into()).unwrap().host,
            "dc01-b.corp.lan"
        );
        let rename = |from: &str, name: &str| Change::RenameGroup {
            group: "g_infra".into(),
            from: from.into(),
            name: name.into(),
        };
        open(&path).apply(rename("Infrastructure", "Servers")).unwrap();
        assert!(matches!(
            library.apply(rename("Infrastructure", "Core")),
            Err(LibraryError::Stale(_))
        ));
        library.apply(rename("Servers", "Core")).unwrap();
        let edit_username = |username: &str| Change::EditCredential {
            original: admin(),
            edited: Credential {
                username: username.into(),
                ..admin()
            },
            password: None,
            today: date!(2026 - 11 - 25),
        };
        open(&path).apply(edit_username(r"CORP\bob")).unwrap();
        assert!(matches!(
            library.apply(edit_username(r"CORP\carol")),
            Err(LibraryError::Stale(_))
        ));
    }

    #[test]
    fn a_credential_is_assigned_only_to_the_endpoint_that_asked() {
        let (_dir, path) = temp_path();
        let mut library = populated(&path, &MemorySecrets::default());
        let lab = ConnectionId::from("n_lab");
        let assign = |host: &str| Change::AssignCredential {
            connection: lab.clone(),
            credential: "c_admin".into(),
            host: host.into(),
            port: 3389,
        };
        // The host changed after the sign-in began: nothing is assigned.
        library.apply(assign("old-lab.corp.lan")).unwrap();
        assert_eq!(library.snapshot().connection(&lab).unwrap().credential, None);
        library.apply(assign("LAB.corp.lan")).unwrap();
        assert_eq!(
            library.snapshot().connection(&lab).unwrap().credential,
            Some("c_admin".into())
        );
        // A Credential chosen since wins.
        library.apply(add_credential("c_other", "Other", "pw")).unwrap();
        library
            .apply(Change::AssignCredential {
                connection: lab.clone(),
                credential: "c_other".into(),
                host: "lab.corp.lan".into(),
                port: 3389,
            })
            .unwrap();
        assert_eq!(
            library.snapshot().connection(&lab).unwrap().credential,
            Some("c_admin".into())
        );
    }

    #[test]
    fn a_credential_whose_file_save_fails_leaves_no_password_behind() {
        let (_dir, path) = temp_path();
        let secrets = MemorySecrets::default();
        let mut library = populated(&path, &secrets);
        let _blocked = block_saves(&path);
        assert!(matches!(
            library.apply(add_credential("c_new", "New", "fresh")),
            Err(LibraryError::Write { .. })
        ));
        assert_eq!(secrets.get(&"c_new".into()), None);
        assert_eq!(library.snapshot().credentials.len(), 1);
    }

    #[test]
    fn a_failed_save_puts_the_previous_password_back() {
        let (_dir, path) = temp_path();
        let secrets = MemorySecrets::default();
        let mut library = populated(&path, &secrets);
        let _blocked = block_saves(&path);
        assert!(library.apply(new_password(admin(), "new")).is_err());
        assert!(library.apply(Change::DeleteCredential("c_admin".into())).is_err());
        assert_eq!(secrets.get(&"c_admin".into()), Some(Secret::from("hunter2".to_owned())));
        assert_eq!(library.snapshot().credentials, vec![admin()]);
    }

    #[test]
    fn a_refused_keychain_deletion_keeps_the_credential() {
        let (_dir, path) = temp_path();
        let secrets = Refusing::default();
        let mut library = populated(&path, &secrets.inner);
        library.secrets = Box::new(secrets.clone());
        secrets.refuse("delete");
        assert!(matches!(
            library.apply(Change::DeleteCredential("c_admin".into())),
            Err(LibraryError::Secrets(_))
        ));
        let snapshot = open(&path).snapshot();
        assert_eq!(snapshot.credentials, vec![admin()]);
        assert_eq!(snapshot.used_by(&"c_admin".into()).len(), 2);
        assert_eq!(library.snapshot(), snapshot);
        assert_eq!(
            secrets.inner.get(&"c_admin".into()),
            Some(Secret::from("hunter2".to_owned()))
        );
    }

    #[test]
    fn a_password_that_cannot_be_put_back_is_reported_with_its_credential() {
        let (_dir, path) = temp_path();
        let secrets = Refusing::default();
        let mut library = populated(&path, &secrets.inner);
        library.secrets = Box::new(secrets.clone());
        let _blocked = block_saves(&path);
        secrets.refuse("delete");
        let error = library.apply(add_credential("c_new", "New", "fresh")).unwrap_err();
        assert!(matches!(&error, LibraryError::Restore { credential, .. } if credential.as_str() == "c_new"));
        let message = error.to_string();
        assert!(
            message.contains("could not be saved") && message.contains("c_new"),
            "{message}"
        );
    }

    #[test]
    fn keys_and_tables_this_version_does_not_know_survive_edits() {
        let (_dir, path) = temp_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            "version = 1\n\n[settings]\nappearance = \"dark\"\nfuture = 1\n\n[[connections]]\nid = \"n_dc01\"\nname = \"dc01\"\nhost = \"dc01.corp.lan\"\ndisplay = { width = 1920 }\nadded = 2026-01-02\n\n[[macros]]\nname = \"later\"\n",
        )
        .unwrap();
        let mut library = open(&path);
        assert_eq!(library.snapshot().settings.appearance, Appearance::Dark);
        let original = library.snapshot().connections[0].clone();
        library
            .apply(Change::EditConnection {
                edited: Connection {
                    name: "dc01 (primary)".into(),
                    ..original.clone()
                },
                original,
            })
            .unwrap();
        library
            .apply(Change::Setting(Setting::RememberCertificates(true)))
            .unwrap();
        library.apply(remember(trusted("fs01.corp.lan", 1))).unwrap();
        let text = fs::read_to_string(&path).unwrap().replace(
            "[[trusted_certificates]]\n",
            "[[trusted_certificates]]\nnote = \"pinned by hand\"\n",
        );
        fs::write(&path, text).unwrap();
        library
            .apply(Change::RememberCertificate {
                certificate: trusted("fs01.corp.lan", 2),
                replacing: Some(trusted("fs01.corp.lan", 1).sha256),
            })
            .unwrap();
        let saved: toml::Table = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            saved["trusted_certificates"][0]["note"].as_str(),
            Some("pinned by hand")
        );
        assert_eq!(
            saved["trusted_certificates"][0]["sha256"].as_str(),
            Some(trusted("fs01.corp.lan", 2).sha256.to_string().as_str())
        );
        assert_eq!(saved["settings"]["future"].as_integer(), Some(1));
        assert_eq!(saved["settings"]["remember_certificates"].as_bool(), Some(true));
        let dc01 = &saved["connections"][0];
        assert_eq!(dc01["name"].as_str(), Some("dc01 (primary)"));
        assert_eq!(dc01["display"]["width"].as_integer(), Some(1920));
        assert_eq!(
            dc01["added"].as_datetime().map(ToString::to_string),
            Some("2026-01-02".into())
        );
        assert_eq!(saved["macros"][0]["name"].as_str(), Some("later"));
    }

    #[test]
    fn remembering_replaces_the_certificate_for_that_host() {
        let (_dir, path) = temp_path();
        let mut library = open(&path);
        library.apply(remember(trusted("fs01.corp.lan", 1))).unwrap();
        library
            .apply(Change::RememberCertificate {
                certificate: trusted("FS01.corp.lan", 2),
                replacing: Some(trusted("fs01.corp.lan", 1).sha256),
            })
            .unwrap();
        assert_eq!(
            library.snapshot().trusted_certificates,
            vec![trusted("FS01.corp.lan", 2)]
        );
    }

    #[test]
    fn remembering_refuses_to_replace_a_certificate_the_person_did_not_see() {
        let (_dir, path) = temp_path();
        let mut stale = open(&path);
        let mut other = open(&path);
        other.apply(remember(trusted("fs01.corp.lan", 1))).unwrap();
        let expected_by_stale = stale.snapshot().trusted_certificate("fs01.corp.lan", 3389).cloned();
        assert_eq!(expected_by_stale, None);
        assert!(matches!(
            stale.apply(remember(trusted("fs01.corp.lan", 2))),
            Err(LibraryError::Conflict { .. })
        ));
        // The refused Library now knows the certificate the other Bezel remembered, so the next
        // Session from it treats any other certificate, including the refused one, as a change.
        let pin = stale.snapshot().trusted_certificate("fs01.corp.lan", 3389).cloned();
        assert_eq!(pin, Some(trusted("fs01.corp.lan", 1)));
        let session = SessionTrust::new(pin);
        assert_eq!(session.decide(&certificate(1)), Decision::Accept);
        assert!(matches!(
            session.decide(&certificate(2)),
            Decision::Ask(Question::Changed { .. })
        ));
        let changed = Change::RememberCertificate {
            certificate: trusted("fs01.corp.lan", 3),
            replacing: Some(trusted("fs01.corp.lan", 9).sha256),
        };
        assert!(matches!(other.apply(changed), Err(LibraryError::Conflict { .. })));
        assert_eq!(
            open(&path).snapshot().trusted_certificates,
            vec![trusted("fs01.corp.lan", 1)]
        );
        stale.apply(remember(trusted("fs01.corp.lan", 1))).unwrap();
    }

    #[test]
    fn forgetting_removes_it() {
        let (_dir, path) = temp_path();
        let mut library = open(&path);
        library.apply(remember(trusted("fs01.corp.lan", 1))).unwrap();
        let forget = |fill| Change::ForgetCertificate {
            host: "fs01.corp.lan".into(),
            port: 3389,
            sha256: trusted("fs01.corp.lan", fill).sha256,
        };
        // The list showed a certificate another Bezel has since replaced.
        assert!(matches!(library.apply(forget(2)), Err(LibraryError::Stale(_))));
        library.apply(forget(1)).unwrap();
        assert!(open(&path).snapshot().trusted_certificates.is_empty());
        library.apply(forget(1)).unwrap();
    }

    #[test]
    fn a_file_that_fails_to_parse_is_refused_and_left_alone() {
        let (_dir, path) = temp_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let broken = "version = 1\n\n[[trusted_certificates]]\nhost = \"fs01\"\nport = \"not a number\"\n";
        fs::write(&path, broken).unwrap();
        match Library::open(&path, MemorySecrets::default()) {
            Err(LibraryError::Parse { line, .. }) => assert_eq!(line, Some(5)),
            other => panic!("expected a parse error, got {:?}", other.err()),
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), broken);
    }

    #[test]
    fn a_newer_version_is_refused() {
        let (_dir, path) = temp_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "version = 2\n").unwrap();
        assert!(matches!(
            Library::open(&path, MemorySecrets::default()),
            Err(LibraryError::Version { found: 2, .. })
        ));
    }

    #[test]
    fn a_change_keeps_edits_made_to_the_file_after_it_was_opened() {
        let (_dir, path) = temp_path();
        let mut library = open(&path);
        let mut other = open(&path);
        other.apply(remember(trusted("dc01.corp.lan", 2))).unwrap();
        fs::write(
            &path,
            format!(
                "{}\n[[groups]]\nid = \"g_7k2m\"\nname = \"Infrastructure\"\n",
                fs::read_to_string(&path).unwrap()
            ),
        )
        .unwrap();
        library.apply(remember(trusted("fs01.corp.lan", 1))).unwrap();
        let snapshot = open(&path).snapshot();
        assert_eq!(snapshot.groups, vec![group("g_7k2m", "Infrastructure")]);
        assert_eq!(
            library.snapshot().trusted_certificates,
            vec![trusted("dc01.corp.lan", 2), trusted("fs01.corp.lan", 1)]
        );
    }

    #[test]
    fn a_change_is_refused_when_the_file_stopped_parsing_after_it_was_opened() {
        let (_dir, path) = temp_path();
        let mut library = open(&path);
        library.apply(remember(trusted("fs01.corp.lan", 1))).unwrap();
        let broken = "version = 1\n[[trusted_certificates]\n";
        fs::write(&path, broken).unwrap();
        assert!(matches!(
            library.apply(remember(trusted("dc01.corp.lan", 2))),
            Err(LibraryError::Parse { .. })
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), broken);
        assert_eq!(
            library.snapshot().trusted_certificates,
            vec![trusted("fs01.corp.lan", 1)]
        );
    }

    #[test]
    fn a_failed_save_keeps_the_previous_state() {
        let (dir, _) = temp_path();
        let path = dir.path().join("bezel.toml");
        let mut library = open(&path);
        fs::create_dir_all(&path).unwrap();
        assert!(library.apply(remember(trusted("fs01.corp.lan", 1))).is_err());
        assert!(library.snapshot().trusted_certificates.is_empty());
    }

    #[test]
    fn random_ids_carry_their_prefix() {
        let a = CredentialId::random();
        assert!(a.as_str().starts_with("c_") && a.as_str().len() == 8, "{a}");
        assert_ne!(a, CredentialId::random());
    }
}
