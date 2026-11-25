//! The window's state and what happens to it: typing a host, signing in, the Session's events
//! and the certificate dialogs.

use std::sync::mpsc;

use bezel_core::address::Address;
use bezel_core::keys::{CommandKey, KeyOutcome, Keyboard, Shortcut};
use bezel_core::library::{Change, Snapshot};
use bezel_core::trust::{Question, TrustedCertificate};
use bezel_core::usernames::Username;
use bezel_rdp::{Auth, CertificateRequest, Ended, Event, Input, Pointer, Session, SessionHandle, State, Target, View};
use rune::{PhysicalInput, TextEdit, Tx, Waker};

use crate::keymap;
use crate::platform;
use crate::store::{Loaded, Reply, Store};

pub struct Bezel {
    waker: Option<Waker>,
    inbox: mpsc::Receiver<Message>,
    outbox: mpsc::Sender<Message>,
    store: Option<Store>,
    pub library: Library,
    pub host: TextEdit,
    pub host_error: Option<String>,
    pub screen: Screen,
    /// The content area the desktop fills, as last laid out.
    pub area: Option<View>,
    /// Why the last Session ended, shown until the next connect.
    pub ended: Option<Ended>,
    /// A problem saving the Library, shown until dismissed.
    pub notice: Option<String>,
    next_session: u64,
    last_username: String,
}

pub enum Library {
    Loading,
    Ready(Snapshot),
    Failed(String),
}

enum Message {
    Session(u64, Event),
    Store(Reply),
}

pub enum Screen {
    Idle,
    SignIn(SignIn),
    Session(Live),
}

pub struct SignIn {
    pub address: Address,
    pub username: TextEdit,
    pub password: TextEdit,
    pub error: Option<String>,
}

pub struct Live {
    id: u64,
    pub address: Address,
    pub username: String,
    pub handle: SessionHandle,
    keyboard: Keyboard,
    pub state: Phase,
    pub pointer: Pointer,
    pub certificate: Option<Prompt>,
    /// The certificate was trusted for this Session only, rather than by the Mac's chain or a
    /// Trusted certificate, so the status line doesn't say "Trusted".
    pub trusted_once: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Connecting,
    Connected { width: u16, height: u16 },
    Reconnecting,
}

pub struct Prompt {
    pub request: CertificateRequest,
    pub remember: bool,
    /// The changed-certificate dialog is asking to confirm "Trust new certificate…".
    pub confirming: bool,
}

/// Used until the content area has been laid out once.
const FALLBACK_VIEW: View = View {
    width: 1024.0,
    height: 640.0,
    scale: 2.0,
};

impl Bezel {
    pub fn new() -> Self {
        let (outbox, inbox) = mpsc::channel();
        Self {
            waker: None,
            inbox,
            outbox,
            store: None,
            library: Library::Loading,
            host: TextEdit::default(),
            host_error: None,
            screen: Screen::Idle,
            area: None,
            ended: None,
            notice: None,
            next_session: 0,
            last_username: String::new(),
        }
    }

    pub fn setup(&mut self, waker: Waker) {
        let outbox = self.outbox.clone();
        self.store = Some(Store::open(
            platform::library_file(),
            move |reply| drop(outbox.send(Message::Store(reply))),
            waker.clone(),
        ));
        self.waker = Some(waker);
    }

    /// Runs before every frame; true when something changed.
    pub fn poll(&mut self, _clock: f64, _tx: &mut Tx) -> bool {
        let mut changed = false;
        while let Ok(message) = self.inbox.try_recv() {
            changed = true;
            match message {
                Message::Store(reply) => self.on_store(reply),
                Message::Session(id, event) => self.on_session(id, event),
            }
        }
        changed
    }

    fn on_store(&mut self, reply: Reply) {
        match reply {
            Reply::Loaded(Loaded::Ready(snapshot)) | Reply::Saved(Ok(snapshot)) => {
                self.library = Library::Ready(snapshot)
            }
            Reply::Loaded(Loaded::Failed(e)) => self.library = Library::Failed(e),
            Reply::Saved(Err(e)) => {
                self.notice = Some(format!(
                    "The certificate is trusted for now but was not remembered. {e}"
                ))
            }
        }
    }

    pub fn submit_host(&mut self) {
        match Address::parse(&self.host.text) {
            Ok(address) => {
                self.host_error = None;
                self.ended = None;
                self.screen = Screen::SignIn(SignIn {
                    address,
                    username: TextEdit::new(self.last_username.clone()),
                    password: TextEdit::default().secure(),
                    error: None,
                });
            }
            Err(e) => self.host_error = Some(e.to_string()),
        }
    }

    pub fn cancel_sign_in(&mut self) {
        self.screen = Screen::Idle;
    }

    pub fn sign_in(&mut self) {
        let Screen::SignIn(sign_in) = &mut self.screen else {
            return;
        };
        let username = match Username::parse(&sign_in.username.text) {
            Ok(username) => username,
            Err(e) => return sign_in.error = Some(e.to_string()),
        };
        let address = sign_in.address.clone();
        let password = sign_in.password.text.clone();
        sign_in.password.clear();
        self.last_username = username.to_string();
        self.connect(address, username, password);
    }

    fn connect(&mut self, address: Address, username: Username, password: String) {
        let trusted_certificate = match &self.library {
            Library::Ready(snapshot) => snapshot.trusted_certificate(&address.host, address.port).cloned(),
            Library::Loading | Library::Failed(_) => None,
        };
        self.next_session += 1;
        let id = self.next_session;
        let (outbox, waker) = (self.outbox.clone(), self.waker.clone());
        let target = Target {
            host: address.host.clone(),
            port: address.port,
            view: self.area.unwrap_or(FALLBACK_VIEW),
            trusted_certificate,
        };
        let display_name = username.to_string();
        let handle = Session::connect(
            target,
            Auth {
                username,
                password: password.into(),
            },
            move |event| {
                let _ = outbox.send(Message::Session(id, event));
                if let Some(wake) = &waker {
                    wake();
                }
            },
        );
        self.ended = None;
        self.screen = Screen::Session(Live {
            id,
            address,
            username: display_name,
            handle,
            keyboard: Keyboard::new(CommandKey::MacShortcuts),
            state: Phase::Connecting,
            pointer: Pointer::Default,
            certificate: None,
            trusted_once: false,
        });
    }

    fn on_session(&mut self, id: u64, event: Event) {
        let Screen::Session(live) = &mut self.screen else {
            return;
        };
        if live.id != id {
            return;
        }
        match event {
            Event::State(State::Connecting) => live.state = Phase::Connecting,
            Event::State(State::Connected { width, height }) => live.state = Phase::Connected { width, height },
            Event::State(State::Reconnecting) => live.state = Phase::Reconnecting,
            Event::State(State::Disconnected(ended)) => {
                if !ended.by_user {
                    self.ended = Some(ended);
                }
                self.screen = Screen::Idle;
            }
            Event::Pointer(pointer) => live.pointer = pointer,
            Event::Certificate(request) => {
                live.certificate = Some(Prompt {
                    request,
                    remember: false,
                    confirming: false,
                })
            }
            Event::SignInFailed { ended, .. } => {
                let address = live.address.clone();
                self.screen = Screen::SignIn(SignIn {
                    address,
                    username: TextEdit::new(self.last_username.clone()),
                    password: TextEdit::default().secure(),
                    error: Some(ended.message),
                });
            }
        }
    }

    pub fn disconnect(&mut self) {
        if let Screen::Session(live) = &self.screen {
            live.handle.disconnect();
        }
        self.screen = Screen::Idle;
    }

    fn prompt(&mut self) -> Option<(&mut Live, Prompt)> {
        let Screen::Session(live) = &mut self.screen else {
            return None;
        };
        let prompt = live.certificate.take()?;
        Some((live, prompt))
    }

    pub fn toggle_remember(&mut self) {
        if let Screen::Session(Live {
            certificate: Some(prompt),
            ..
        }) = &mut self.screen
        {
            prompt.remember = !prompt.remember;
        }
    }

    /// "Trust and connect" for a first use, or the confirmed "Trust new certificate" for a change.
    pub fn trust(&mut self) {
        let Some((live, prompt)) = self.prompt() else { return };
        let (host, port) = (prompt.request.host.clone(), prompt.request.port);
        let remember = match &prompt.request.question {
            Question::FirstUse(certificate) => prompt.remember.then_some(certificate),
            Question::Changed { presented, .. } => Some(presented),
        };
        let change = remember.map(|c| Change::RememberCertificate(TrustedCertificate::new(&host, port, c, today())));
        live.trusted_once = change.is_none();
        prompt.request.trust();
        let Some(change) = change else { return };
        match (&self.library, &self.store) {
            // A change made while the file is still loading waits for it on the store's thread.
            (Library::Ready(_) | Library::Loading, Some(store)) => store.apply(change),
            (Library::Failed(e), _) => {
                self.notice = Some(format!(
                    "The certificate is trusted for now but was not remembered. {e}"
                ))
            }
            _ => self.notice = Some("The certificate is trusted for now but was not remembered.".into()),
        }
    }

    pub fn confirm_change(&mut self, confirming: bool) {
        if let Screen::Session(Live {
            certificate: Some(prompt),
            ..
        }) = &mut self.screen
        {
            prompt.confirming = confirming;
        }
    }

    pub fn refuse_certificate(&mut self) {
        if let Some((_, prompt)) = self.prompt() {
            prompt.request.cancel();
        }
    }

    pub fn layout_area(&mut self, rect: [f32; 4], tx: &mut Tx) {
        let view = View {
            width: rect[2],
            height: rect[3],
            scale: tx.scale(),
        };
        if self.area == Some(view) {
            return;
        }
        self.area = Some(view);
        if let Screen::Session(live) = &self.screen {
            live.handle.resize(view);
        }
    }

    pub fn input(&self, input: Input) {
        if let Screen::Session(live) = &self.screen {
            live.handle.input(input);
        }
    }

    /// Keys pressed over the desktop. True when the Session took the key.
    pub fn physical(&mut self, event: &PhysicalInput) -> bool {
        let Screen::Session(live) = &mut self.screen else {
            return false;
        };
        match event {
            PhysicalInput::Key { key, pressed, .. } => {
                let Some(scancode) = keymap::scancode(*key) else {
                    return false;
                };
                match live.keyboard.key(scancode, *pressed) {
                    KeyOutcome::Send(ops) => {
                        if !ops.is_empty() {
                            live.handle.input(Input::Keys(ops));
                        }
                        true
                    }
                    KeyOutcome::Shortcut(Shortcut::Close) => {
                        self.disconnect();
                        true
                    }
                    // Tabs, the palette and Settings arrive in later milestones; until then
                    // Rune and the menus handle these keys (⌘Q quits).
                    KeyOutcome::Shortcut(_) => false,
                }
            }
            PhysicalInput::Modifiers(_) => false,
            PhysicalInput::FocusLost => {
                live.keyboard.release_all();
                live.handle.input(Input::ReleaseAll);
                false
            }
        }
    }
}

/// For fixture scenes: a Session with nothing behind it.
pub fn fixture_session(address: Address, state: Phase, certificate: Option<CertificateRequest>) -> Live {
    Live {
        id: 0,
        address,
        username: r"CORP\admin.max".into(),
        handle: SessionHandle::detached(),
        keyboard: Keyboard::new(CommandKey::MacShortcuts),
        state,
        pointer: Pointer::Default,
        certificate: certificate.map(|request| Prompt {
            request,
            remember: false,
            confirming: false,
        }),
        trusted_once: false,
    }
}

fn today() -> time::Date {
    time::OffsetDateTime::now_utc().date()
}
