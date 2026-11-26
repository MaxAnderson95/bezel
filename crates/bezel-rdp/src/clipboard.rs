//! The text clipboard between the Mac and the Session. The app hands over the Mac's text
//! whenever it changes; the CLIPRDR backend offers it to Windows and sends it when Windows
//! pastes. When Windows copies text, the backend fetches it at once and it becomes a
//! `Clipboard` event. Text only: images and files are left out (the spec's non-goals).
//!
//! One `Clipboard` lives as long as the Session; IronRDP builds a backend for each connection,
//! reconnects included, and all of them share the Mac's text.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ironrdp_client::rdp::RdpInputSender;
use ironrdp_cliprdr::backend::{ClipboardMessage, CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest, FileContentsResponse,
    FormatDataRequest, FormatDataResponse, LockDataId,
};
use tracing::warn;

use crate::Event;
use crate::engine::Emit;

pub(crate) struct Clipboard {
    shared: Arc<Mutex<Shared>>,
    emit: Emit,
}

#[derive(Default)]
struct Shared {
    /// The Mac's text as last handed over; `None` when it holds no text.
    text: Option<String>,
    /// Counts the texts handed over, so each reaches Windows once per channel.
    version: u64,
    /// The current connection's channel, once its backend is built.
    channel: Option<Channel>,
}

struct Channel {
    sender: RdpInputSender,
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
            let _ = channel
                .sender
                .send_clipboard(ClipboardMessage::SendInitiateCopy(formats));
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

    /// The backend factory for one `RdpClient`, whose input `sender` carries clipboard messages.
    pub fn factory(&self, sender: RdpInputSender) -> Box<dyn CliprdrBackendFactory + Send> {
        Box::new(Factory {
            shared: self.shared.clone(),
            emit: self.emit.clone(),
            sender,
        })
    }
}

struct Factory {
    shared: Arc<Mutex<Shared>>,
    emit: Emit,
    sender: RdpInputSender,
}

impl CliprdrBackendFactory for Factory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        lock(&self.shared).channel = Some(Channel {
            sender: self.sender.clone(),
            ready: false,
            offered: 0,
        });
        Box::new(Backend {
            shared: self.shared.clone(),
            emit: self.emit.clone(),
            sender: self.sender.clone(),
        })
    }
}

struct Backend {
    shared: Arc<Mutex<Shared>>,
    emit: Emit,
    sender: RdpInputSender,
}

ironrdp_core::impl_as_any!(Backend);

impl fmt::Debug for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Backend")
    }
}

impl Backend {
    fn send(&self, message: ClipboardMessage) {
        let _ = self.sender.send_clipboard(message);
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
        match response.to_unicode_string() {
            Ok(text) => (self.emit)(Event::Clipboard(Some(text))),
            Err(e) => warn!(error = %e, "the text Windows copied could not be read"),
        }
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
