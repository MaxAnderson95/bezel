//! The text clipboard between the Mac and the Session. The app hands over the Mac's text
//! whenever it changes; the CLIPRDR backend offers it to Windows and sends it when Windows
//! pastes. When Windows copies text, the backend fetches it at once and it becomes a
//! `Clipboard` event. Text only: images and files are left out (the spec's non-goals).
//!
//! One `Clipboard` lives as long as the Session; IronRDP builds a backend for each connection,
//! reconnects included, and all of them share the text the two clipboards last held.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse,
    FormatDataRequest, FormatDataResponse, LockDataId,
};
use tracing::warn;

use crate::Event;
use crate::engine::Emit;

/// Hands a clipboard message to the connection's CLIPRDR channel.
pub(crate) type Post = Arc<dyn Fn(ClipboardMessage) + Send + Sync>;

pub(crate) struct Clipboard {
    shared: Arc<Mutex<Shared>>,
    emit: Emit,
}

#[derive(Default)]
struct Shared {
    /// The text both clipboards last held: the Mac's as handed over, or what Windows copied.
    /// `None` when that was something other than text.
    text: Option<String>,
    /// Counts the texts, so each reaches Windows once per channel.
    version: u64,
    /// The current connection's channel, once its backend is built.
    channel: Option<Channel>,
}

struct Channel {
    post: Post,
    /// The channel took its first format list, so a new one can go at any time.
    ready: bool,
    /// The `version` Windows was last offered.
    offered: u64,
}

impl Shared {
    /// Offers the Mac's text to Windows, or nothing when it holds none.
    fn offer(&mut self) {
        let version = self.version;
        let formats = match self.text.as_deref() {
            Some(text) if !text.is_empty() => vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)],
            _ => Vec::new(),
        };
        if let Some(channel) = &mut self.channel {
            channel.offered = version;
            (channel.post)(ClipboardMessage::SendInitiateCopy(formats));
        }
    }
}

impl Clipboard {
    pub fn new(emit: Emit) -> Self {
        Self {
            shared: Arc::default(),
            emit,
        }
    }

    /// The Mac's clipboard changed. Windows is offered it now if the channel is ready, or once
    /// it is.
    pub fn set(&self, text: Option<String>) {
        let mut shared = lock(&self.shared);
        shared.text = text;
        shared.version += 1;
        if shared.channel.as_ref().is_some_and(|c| c.ready) {
            shared.offer();
        }
    }

    /// The backend factory for one `RdpClient`, whose CLIPRDR channel takes messages through
    /// `post`.
    pub fn factory(&self, post: Post) -> Box<dyn CliprdrBackendFactory + Send> {
        Box::new(Factory {
            shared: self.shared.clone(),
            emit: self.emit.clone(),
            post,
        })
    }
}

struct Factory {
    shared: Arc<Mutex<Shared>>,
    emit: Emit,
    post: Post,
}

impl CliprdrBackendFactory for Factory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        lock(&self.shared).channel = Some(Channel {
            post: self.post.clone(),
            ready: false,
            offered: 0,
        });
        Box::new(Backend {
            shared: self.shared.clone(),
            emit: self.emit.clone(),
            post: self.post.clone(),
        })
    }
}

struct Backend {
    shared: Arc<Mutex<Shared>>,
    emit: Emit,
    post: Post,
}

ironrdp_core::impl_as_any!(Backend);

impl fmt::Debug for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Backend")
    }
}

impl Backend {
    fn send(&self, message: ClipboardMessage) {
        (self.post)(message);
    }
}

impl CliprdrBackend for Backend {
    fn temporary_directory(&self) -> &str {
        ".cliprdr"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_ready(&mut self) {
        let mut shared = lock(&self.shared);
        let version = shared.version;
        let Some(channel) = &mut shared.channel else { return };
        channel.ready = true;
        // The Mac's clipboard changed while the channel started.
        if channel.offered != version {
            shared.offer();
        }
    }

    fn on_request_format_list(&mut self) {
        lock(&self.shared).offer();
    }

    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}

    fn on_remote_copy(&mut self, formats: &[ClipboardFormat]) {
        if formats.iter().any(|f| f.id() == ClipboardFormatId::CF_UNICODETEXT) {
            self.send(ClipboardMessage::SendInitiatePaste(ClipboardFormatId::CF_UNICODETEXT));
        } else {
            (self.emit)(Event::Clipboard(None));
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let text = lock(&self.shared).text.clone();
        let response = match text {
            Some(text) if request.format == ClipboardFormatId::CF_UNICODETEXT => {
                FormatDataResponse::new_unicode_string(&text)
            }
            _ => FormatDataResponse::new_error(),
        };
        self.send(ClipboardMessage::SendFormatData(response));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if response.is_error() {
            return warn!("Windows could not hand over the text it copied");
        }
        let text = match response.to_unicode_string() {
            Ok(text) => text,
            Err(e) => return warn!(error = %e, "the text Windows copied could not be read"),
        };
        // Windows has it already; a later channel, after a reconnect, offers it and serves it
        // instead of the Mac's text from before.
        let mut shared = lock(&self.shared);
        shared.text = Some(text.clone());
        shared.version += 1;
        let version = shared.version;
        if let Some(channel) = &mut shared.channel {
            channel.offered = version;
        }
        drop(shared);
        (self.emit)(Event::Clipboard(Some(text)));
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        self.send(ClipboardMessage::SendFileContentsResponse(
            FileContentsResponse::new_error(request.stream_id),
        ));
    }

    fn on_file_contents_response(&mut self, _: FileContentsResponse<'_>) {}

    fn on_lock(&mut self, _: LockDataId) {}

    fn on_unlock(&mut self, _: LockDataId) {}
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a backend sent its channel: the formats it offered, or the text it served.
    #[derive(Debug, PartialEq)]
    enum Sent {
        Offered(Vec<u32>),
        Served(Option<String>),
        Other,
    }

    fn sent(message: ClipboardMessage) -> Sent {
        match message {
            ClipboardMessage::SendInitiateCopy(formats) => {
                Sent::Offered(formats.iter().map(|f| f.id().value()).collect())
            }
            ClipboardMessage::SendFormatData(response) => {
                Sent::Served((!response.is_error()).then(|| response.to_unicode_string().unwrap()))
            }
            _ => Sent::Other,
        }
    }

    #[test]
    fn text_windows_copied_is_what_a_new_channel_offers_and_serves() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let messages = Arc::new(Mutex::new(Vec::new()));
        let clipboard = Clipboard::new({
            let events = events.clone();
            Arc::new(move |event| {
                if let Event::Clipboard(text) = event {
                    events.lock().unwrap().push(text);
                }
            })
        });
        let factory = clipboard.factory({
            let messages = messages.clone();
            Arc::new(move |message| messages.lock().unwrap().push(sent(message)))
        });
        let take = || std::mem::take(&mut *messages.lock().unwrap());
        let text = ClipboardFormatId::CF_UNICODETEXT.value();

        clipboard.set(Some("copied on the Mac".into()));
        let mut first = factory.build_cliprdr_backend();
        first.on_request_format_list();
        first.on_ready();
        assert_eq!(take(), vec![Sent::Offered(vec![text])]);
        // Windows copies, and its text is fetched for the Mac, without offering it back.
        first.on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]);
        assert_eq!(take(), vec![Sent::Other], "a paste request");
        first.on_format_data_response(FormatDataResponse::new_unicode_string("copied in Windows"));
        assert_eq!(take(), vec![]);
        assert_eq!(*events.lock().unwrap(), vec![Some("copied in Windows".to_owned())]);
        // A reconnect's channel offers and serves Windows' text, not the Mac's from before.
        let mut second = factory.build_cliprdr_backend();
        second.on_request_format_list();
        second.on_ready();
        second.on_format_data_request(FormatDataRequest {
            format: ClipboardFormatId::CF_UNICODETEXT,
        });
        assert_eq!(
            take(),
            vec![
                Sent::Offered(vec![text]),
                Sent::Served(Some("copied in Windows".into()))
            ]
        );
    }
}
