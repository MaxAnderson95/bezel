//! The Library on a thread of its own, so reading and saving `bezel.toml` never runs on the UI
//! thread.

use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use bezel_core::library::{Change, Library, Snapshot};
use rune::Waker;

pub enum Loaded {
    Ready(Snapshot),
    /// The file could not be used; nothing will be saved to it.
    Failed(String),
}

/// Identifies one change, so its reply reaches the request that made it.
pub type ChangeId = u64;

pub enum Reply {
    Loaded(Loaded),
    Saved(ChangeId, Result<Snapshot, String>),
}

pub struct Store {
    changes: mpsc::Sender<(ChangeId, Change)>,
}

impl Store {
    /// Opens the Library in the background; replies arrive through `reply`. If the thread can't
    /// start, the load is reported as failed at once.
    pub fn open(path: PathBuf, reply: impl Fn(Reply) + Send + Sync + 'static, wake: Waker) -> Self {
        let send: Arc<dyn Fn(Reply) + Send + Sync> = Arc::new(move |r| {
            reply(r);
            wake();
        });
        let (changes, receiver) = mpsc::channel::<(ChangeId, Change)>();
        let worker = send.clone();
        let spawned = std::thread::Builder::new().name("library".into()).spawn(move || {
            let mut library = match Library::open(&path) {
                Ok(library) => {
                    worker(Reply::Loaded(Loaded::Ready(library.snapshot())));
                    library
                }
                Err(e) => {
                    tracing::error!(error = %e, "library not loaded");
                    return worker(Reply::Loaded(Loaded::Failed(e.to_string())));
                }
            };
            for (id, change) in receiver {
                let saved = library
                    .apply(change)
                    .map(|()| library.snapshot())
                    .map_err(|e| e.to_string());
                if let Err(e) = &saved {
                    tracing::error!(error = %e, "library not saved");
                }
                worker(Reply::Saved(id, saved));
            }
        });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "library thread not started");
            send(Reply::Loaded(Loaded::Failed(format!(
                "Bezel could not open its data: {e}"
            ))));
        }
        Self { changes }
    }

    pub fn apply(&self, id: ChangeId, change: Change) {
        let _ = self.changes.send((id, change));
    }
}
