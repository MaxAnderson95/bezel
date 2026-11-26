//! One Session with a Windows machine. This crate owns everything IronRDP: the threads and
//! runtime, configuration, the Retina recipe, resize debouncing, certificate prompts, the
//! framebuffer, reconnecting, the text clipboard and audio. No IronRDP type crosses this
//! interface.

mod audio;
mod clipboard;
mod engine;
mod failure;
mod framebuffer;
mod input;
mod retina;

use std::fmt;
use std::sync::Arc;

use bezel_core::keys::KeyOp;
use bezel_core::trust::{Question, TrustedCertificate};
use bezel_core::usernames::Username;
use rune_image::Pixels;
use tokio::sync::{mpsc, oneshot};

pub use failure::Ended;

/// The IronRDP revision pinned in the workspace `Cargo.toml`, for Copy details.
pub const IRONRDP_REVISION: &str = "a0ad31cb864a25ec077034db2d4ced19575d5c20";

/// Where to connect, and the content area the remote desktop fills.
#[derive(Clone, Debug)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub view: View,
    /// The Trusted certificate for this host and port, if one is remembered.
    pub trusted_certificate: Option<TrustedCertificate>,
}

/// A content area in points and the backing scale of the display showing it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub width: f32,
    pub height: f32,
    pub scale: f32,
}

pub struct Auth {
    pub username: Username,
    pub password: Password,
}

/// A password that never prints.
pub struct Password(String);

impl From<String> for Password {
    fn from(password: String) -> Self {
        Self(password)
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(…)")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// The pointer over the desktop, as fractions of its width and height from the top left.
    PointerMove {
        x: f32,
        y: f32,
    },
    Button {
        button: MouseButton,
        pressed: bool,
    },
    /// A wheel or trackpad scroll in points, positive moving content right and down.
    Wheel {
        dx: f32,
        dy: f32,
    },
    Keys(Vec<KeyOp>),
    /// Lets go of every key and button, for when the desktop loses focus.
    ReleaseAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

pub enum Event {
    State(State),
    Pointer(Pointer),
    /// The connection waits until the request is answered or dropped (which cancels).
    Certificate(CertificateRequest),
    /// The server rejected the sign-in. The Session has ended.
    SignInFailed {
        wrong_password: bool,
        ended: Ended,
    },
    /// Windows copied: its text for the Mac's clipboard, or `None` for something other than
    /// text, which Bezel does not share and which leaves the Mac's clipboard as it is.
    Clipboard(Option<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Connecting,
    /// Sent again whenever the remote desktop changes size.
    Connected {
        width: u16,
        height: u16,
    },
    /// Reconnecting: after the link dropped, for about 30 seconds with the server's
    /// auto-reconnect cookie, or at a new size because the server cannot resize in place.
    Reconnecting,
    Disconnected(Ended),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pointer {
    Default,
    Hidden,
    Shape(Arc<PointerShape>),
}

/// The remote pointer in desktop pixels, straight-alpha RGBA.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct PointerShape {
    pub width: u16,
    pub height: u16,
    pub hotspot: (u16, u16),
    pub rgba: Vec<u8>,
}

pub struct CertificateRequest {
    pub host: String,
    pub port: u16,
    pub question: Question,
    reply: oneshot::Sender<Answer>,
}

impl CertificateRequest {
    /// Accepts the certificate. With `remember` (always the case for a changed certificate,
    /// which replaces the Trusted one) it becomes the certificate this Session expects from
    /// then on, so a different one on a later handshake is asked about as a change. Without,
    /// it is the Session's one exception, for its reconnects included.
    pub fn trust(self, remember: bool) {
        let _ = self.reply.send(Answer::Trust { remember });
    }

    pub fn cancel(self) {
        let _ = self.reply.send(Answer::Cancel);
    }

    /// A request no Session waits on, for fixture scenes.
    pub fn detached(host: impl Into<String>, port: u16, question: Question) -> Self {
        Self {
            host: host.into(),
            port,
            question,
            reply: oneshot::channel().0,
        }
    }
}

pub(crate) enum Answer {
    Trust { remember: bool },
    Cancel,
}

enum Command {
    Input(Input),
    Resize(View),
    CtrlAltDel,
    Clipboard(Option<String>),
    Mute(bool),
    Disconnect,
}

pub struct Session;

impl Session {
    /// Starts connecting on a thread of its own and returns at once. `events` is called from
    /// that thread.
    pub fn connect(target: Target, auth: Auth, events: impl Fn(Event) + Send + Sync + 'static) -> SessionHandle {
        let (commands, receiver) = mpsc::unbounded_channel();
        let framebuffer = Pixels::new();
        engine::spawn(target, auth, std::sync::Arc::new(events), receiver, framebuffer.clone());
        SessionHandle { commands, framebuffer }
    }
}

/// Dropping the handle disconnects.
pub struct SessionHandle {
    commands: mpsc::UnboundedSender<Command>,
    framebuffer: Pixels,
}

impl SessionHandle {
    /// A handle with no Session behind it, for fixture scenes.
    pub fn detached() -> Self {
        Self {
            commands: mpsc::unbounded_channel().0,
            framebuffer: Pixels::new(),
        }
    }

    /// The remote desktop, updated in place as frames arrive; drawing it needs no event.
    pub fn framebuffer(&self) -> &Pixels {
        &self.framebuffer
    }

    pub fn input(&self, input: Input) {
        let _ = self.commands.send(Command::Input(input));
    }

    /// Call on every layout change. The desktop follows about 250 ms after the changes stop.
    pub fn resize(&self, view: View) {
        let _ = self.commands.send(Command::Resize(view));
    }

    pub fn send_ctrl_alt_del(&self) {
        let _ = self.commands.send(Command::CtrlAltDel);
    }

    /// The Mac's clipboard holds this text now, or no text. Windows is offered it to paste,
    /// on every reconnect too.
    pub fn set_clipboard(&self, text: Option<String>) {
        let _ = self.commands.send(Command::Clipboard(text));
    }

    /// Silences the Session's audio, or plays it again.
    pub fn set_muted(&self, muted: bool) {
        let _ = self.commands.send(Command::Mute(muted));
    }

    /// Ends the Session; Windows keeps the user signed in.
    pub fn disconnect(&self) {
        let _ = self.commands.send(Command::Disconnect);
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        self.disconnect();
    }
}
