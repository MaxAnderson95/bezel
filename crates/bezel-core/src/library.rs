//! The persistent model in `bezel.toml`. M1 keeps Trusted certificates only; any other table
//! in the file is carried through untouched so a later Bezel's data survives a save.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::trust::{Fingerprint, TrustedCertificate};

const VERSION: u32 = 1;

pub struct Library {
    path: PathBuf,
    file: File,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub trusted_certificates: Vec<TrustedCertificate>,
}

impl Snapshot {
    pub fn trusted_certificate(&self, host: &str, port: u16) -> Option<&TrustedCertificate> {
        self.trusted_certificates.iter().find(|t| t.is_for(host, port))
    }
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
    ForgetCertificate {
        host: String,
        port: u16,
    },
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
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    trusted_certificates: Vec<TrustedCertificate>,
    #[serde(flatten)]
    rest: toml::Table,
}

impl Library {
    /// A missing file is an empty Library; the file is created by the first change. A file
    /// that fails to parse is an error, and no Library exists to overwrite it.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, LibraryError> {
        let path = path.into();
        let file = read(&path)?;
        Ok(Self { path, file })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            trusted_certificates: self.file.trusted_certificates.clone(),
        }
    }

    /// Applies the change to the file as it is on disk now and saves it. A hand edit or another
    /// Bezel may have changed the file since it was opened, so the change never replaces it
    /// with an older copy, and a file that no longer parses is refused rather than overwritten.
    /// On any error the file is left as it was. The Library keeps its previous state too, except
    /// after a `Conflict`, when it takes the certificates now on disk so the next Session
    /// expects what another Bezel remembered.
    pub fn apply(&mut self, change: Change) -> Result<(), LibraryError> {
        let _lock = lock(&self.path)?;
        let mut file = read(&self.path)?;
        let certificates = &mut file.trusted_certificates;
        match change {
            Change::RememberCertificate { certificate, replacing } => {
                let current = certificates
                    .iter()
                    .find(|t| t.is_for(&certificate.host, certificate.port));
                let on_disk = current.map(|t| t.sha256);
                if on_disk != replacing && on_disk != Some(certificate.sha256) {
                    self.file = file;
                    return Err(LibraryError::Conflict {
                        host: certificate.host,
                        port: certificate.port,
                    });
                }
                certificates.retain(|t| !t.is_for(&certificate.host, certificate.port));
                certificates.push(certificate);
            }
            Change::ForgetCertificate { host, port } => certificates.retain(|t| !t.is_for(&host, port)),
        }
        let error = |source| LibraryError::Write {
            path: self.path.clone(),
            source,
        };
        let text = toml::to_string(&file).map_err(|e| error(io::Error::other(e)))?;
        write_atomically(&self.path, text.as_bytes()).map_err(error)?;
        self.file = file;
        Ok(())
    }
}

fn read(path: &Path) -> Result<File, LibraryError> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(File {
                version: VERSION,
                trusted_certificates: Vec::new(),
                rest: toml::Table::new(),
            });
        }
        Err(source) => {
            return Err(LibraryError::Read {
                path: path.into(),
                source,
            });
        }
    };
    let file: File = toml::from_str(&source).map_err(|e| LibraryError::Parse {
        line: e.span().map(|span| source[..span.start].lines().count().max(1)),
        message: e.message().to_owned(),
        path: path.into(),
    })?;
    if file.version != VERSION {
        return Err(LibraryError::Version {
            path: path.into(),
            found: file.version,
        });
    }
    Ok(file)
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
}

#[cfg(test)]
mod tests {
    use time::macros::date;

    use super::*;
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

    fn temp_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Bezel").join("bezel.toml");
        (dir, path)
    }

    #[test]
    fn a_missing_file_is_an_empty_library_and_nothing_is_written() {
        let (_dir, path) = temp_path();
        let library = Library::open(&path).unwrap();
        assert_eq!(library.snapshot(), Snapshot::default());
        assert!(!path.exists());
    }

    #[test]
    fn remembered_certificates_round_trip() {
        let (_dir, path) = temp_path();
        let mut library = Library::open(&path).unwrap();
        library.apply(remember(trusted("fs01.corp.lan", 1))).unwrap();
        library.apply(remember(trusted("dc01.corp.lan", 2))).unwrap();
        let reopened = Library::open(&path).unwrap().snapshot();
        assert_eq!(reopened, library.snapshot());
        assert_eq!(
            reopened.trusted_certificate("FS01.corp.lan", 3389),
            Some(&trusted("fs01.corp.lan", 1))
        );
    }

    #[test]
    fn writes_the_documented_format() {
        let (_dir, path) = temp_path();
        Library::open(&path)
            .unwrap()
            .apply(remember(trusted("fs01.corp.lan", 1)))
            .unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("version = 1\n"), "{text}");
        assert!(text.contains("[[trusted_certificates]]"), "{text}");
        assert!(text.contains("not_after = 2027-03-14\n"), "{text}");
        assert!(text.contains("remembered = 2026-10-03\n"), "{text}");
    }

    #[test]
    fn remembering_replaces_the_certificate_for_that_host() {
        let (_dir, path) = temp_path();
        let mut library = Library::open(&path).unwrap();
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
        let mut stale = Library::open(&path).unwrap();
        let mut other = Library::open(&path).unwrap();
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
            Library::open(&path).unwrap().snapshot().trusted_certificates,
            vec![trusted("fs01.corp.lan", 1)]
        );
        stale.apply(remember(trusted("fs01.corp.lan", 1))).unwrap();
    }

    #[test]
    fn forgetting_removes_it() {
        let (_dir, path) = temp_path();
        let mut library = Library::open(&path).unwrap();
        library.apply(remember(trusted("fs01.corp.lan", 1))).unwrap();
        library
            .apply(Change::ForgetCertificate {
                host: "fs01.corp.lan".into(),
                port: 3389,
            })
            .unwrap();
        assert!(Library::open(&path).unwrap().snapshot().trusted_certificates.is_empty());
    }

    #[test]
    fn a_file_that_fails_to_parse_is_refused_and_left_alone() {
        let (_dir, path) = temp_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let broken = "version = 1\n\n[[trusted_certificates]]\nhost = \"fs01\"\nport = \"not a number\"\n";
        fs::write(&path, broken).unwrap();
        match Library::open(&path) {
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
            Library::open(&path),
            Err(LibraryError::Version { found: 2, .. })
        ));
    }

    #[test]
    fn tables_this_version_does_not_know_survive_a_save() {
        let (_dir, path) = temp_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "version = 1\n\n[settings]\nappearance = \"dark\"\n\n[[groups]]\nid = \"g_7k2m\"\nname = \"Infrastructure\"\n").unwrap();
        Library::open(&path)
            .unwrap()
            .apply(remember(trusted("fs01.corp.lan", 1)))
            .unwrap();
        let saved: toml::Table = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["settings"]["appearance"].as_str(), Some("dark"));
        assert_eq!(saved["groups"][0]["name"].as_str(), Some("Infrastructure"));
        assert_eq!(saved["trusted_certificates"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn a_change_keeps_edits_made_to_the_file_after_it_was_opened() {
        let (_dir, path) = temp_path();
        let mut library = Library::open(&path).unwrap();
        let mut other = Library::open(&path).unwrap();
        other.apply(remember(trusted("dc01.corp.lan", 2))).unwrap();
        fs::write(
            &path,
            format!(
                "{}\n[settings]\nappearance = \"dark\"\n",
                fs::read_to_string(&path).unwrap()
            ),
        )
        .unwrap();
        library.apply(remember(trusted("fs01.corp.lan", 1))).unwrap();
        let saved: toml::Table = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["settings"]["appearance"].as_str(), Some("dark"));
        assert_eq!(
            library.snapshot().trusted_certificates,
            vec![trusted("dc01.corp.lan", 2), trusted("fs01.corp.lan", 1)]
        );
    }

    #[test]
    fn a_change_is_refused_when_the_file_stopped_parsing_after_it_was_opened() {
        let (_dir, path) = temp_path();
        let mut library = Library::open(&path).unwrap();
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
        // A directory where the file should be makes the rename fail.
        let path = dir.path().join("bezel.toml");
        let mut library = Library::open(&path).unwrap();
        fs::create_dir_all(&path).unwrap();
        assert!(library.apply(remember(trusted("fs01.corp.lan", 1))).is_err());
        assert!(library.snapshot().trusted_certificates.is_empty());
    }
}
