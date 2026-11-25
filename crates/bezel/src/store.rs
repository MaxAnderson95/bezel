//! The Library on a thread of its own, so reading and saving `bezel.toml` never runs on the UI
//! thread.

use std::path::PathBuf;
use std::sync::mpsc;

use bezel_core::library::{Change, Library, Snapshot};
use rune::Waker;

pub enum Loaded {
    Ready(Snapshot),
    /// The file could not be used; nothing will be saved to it.
    Failed(String),
}

pub enum Reply {
    Loaded(Loaded),
    Saved(Result<Snapshot, String>),
}

pub struct Store {
    changes: mpsc::Sender<Change>,
}

impl Store {
    /// Opens the Library in the background; replies arrive through `reply`.
    pub fn open(path: PathBuf, reply: impl Fn(Reply) + Send + 'static, wake: Waker) -> Self {
        let (changes, receiver) = mpsc::channel::<Change>();
        let spawned = std::thread::Builder::new().name("library".into()).spawn(move || {
            let send = |r| {
                reply(r);
                wake();
            };
            let mut library = match Library::open(&path) {
                Ok(library) => {
                    send(Reply::Loaded(Loaded::Ready(library.snapshot())));
                    library
                }
                Err(e) => {
                    tracing::error!(error = %e, "library not loaded");
                    return send(Reply::Loaded(Loaded::Failed(e.to_string())));
                }
            };
            for change in receiver {
                let saved = library
                    .apply(change)
                    .map(|()| library.snapshot())
                    .map_err(|e| e.to_string());
                if let Err(e) = &saved {
                    tracing::error!(error = %e, "library not saved");
                }
                send(Reply::Saved(saved));
            }
        });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "library thread not started");
        }
        Self { changes }
    }

    pub fn apply(&self, change: Change) {
        let _ = self.changes.send(change);
    }
}
