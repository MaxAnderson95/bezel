//! The Library on a thread of its own, so reading and saving `bezel.toml` and every Keychain
//! call stay off the UI thread.

use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use bezel_core::library::{Change, CredentialId, Library, Snapshot};
use bezel_core::secrets::{Secret, Secrets};
use rune::Waker;

pub enum Loaded {
    Ready(Snapshot),
    /// The file could not be used; nothing will be saved to it.
    Failed(String),
}

/// Identifies one request, so its reply reaches what made it.
pub type RequestId = u64;

pub enum Reply {
    Loaded(Loaded),
    /// The Library as it stands after the change, which a refused change can also update
    /// (another Bezel changed the file), and the error if it was refused.
    Saved {
        id: RequestId,
        snapshot: Snapshot,
        error: Option<String>,
    },
    /// A Credential's password; `None` when the Keychain has none for it.
    Password {
        id: RequestId,
        password: Result<Option<Secret>, String>,
    },
}

enum Command {
    Apply(RequestId, Change),
    Password(RequestId, CredentialId),
}

pub struct Store {
    commands: mpsc::Sender<Command>,
}

impl Store {
    /// Opens the Library in the background; replies arrive through `reply`, in the order the
    /// requests were made and always after `Loaded`. If the thread can't start, the load is
    /// reported as failed at once.
    pub fn open(
        path: PathBuf,
        secrets: impl Secrets + 'static,
        reply: impl Fn(Reply) + Send + Sync + 'static,
        wake: Waker,
    ) -> Self {
        let send: Arc<dyn Fn(Reply) + Send + Sync> = Arc::new(move |r| {
            reply(r);
            wake();
        });
        let (commands, receiver) = mpsc::channel::<Command>();
        let worker = send.clone();
        let spawned = std::thread::Builder::new().name("library".into()).spawn(move || {
            let mut library = match Library::open(&path, secrets) {
                Ok(library) => {
                    worker(Reply::Loaded(Loaded::Ready(library.snapshot())));
                    library
                }
                Err(e) => {
                    tracing::error!(error = %e, "library not loaded");
                    return worker(Reply::Loaded(Loaded::Failed(e.to_string())));
                }
            };
            for command in receiver {
                worker(match command {
                    Command::Apply(id, change) => {
                        let error = library.apply(change).err().map(|e| e.to_string());
                        if let Some(e) = &error {
                            tracing::error!(error = %e, "library not saved");
                        }
                        Reply::Saved {
                            id,
                            snapshot: library.snapshot(),
                            error,
                        }
                    }
                    Command::Password(id, credential) => {
                        let password = library.password(&credential).map_err(|e| e.to_string());
                        if let Err(e) = &password {
                            tracing::error!(error = %e, %credential, "password not read");
                        }
                        Reply::Password { id, password }
                    }
                });
            }
        });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "library thread not started");
            send(Reply::Loaded(Loaded::Failed(format!(
                "Bezel could not open its data: {e}"
            ))));
        }
        Self { commands }
    }

    pub fn apply(&self, id: RequestId, change: Change) {
        let _ = self.commands.send(Command::Apply(id, change));
    }

    pub fn password(&self, id: RequestId, credential: CredentialId) {
        let _ = self.commands.send(Command::Password(id, credential));
    }
}
