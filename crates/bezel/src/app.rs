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

use crate::keymap::{self, Translated};
use crate::platform;
use crate::store::{ChangeId, Loaded, Reply, Store};

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
    /// A sign-in waiting for the Library to load, so its Trusted certificate is known before
    /// the Session decides whether a certificate is new or changed.
    waiting: Option<(Address, Username, String)>,
    /// The app menu's Hide shortcuts no longer claim ⌘H and ⌥⌘H.
    menus_ready: bool,
    next_change: ChangeId,
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
    pub trust: Trust,
}

/// How this Session's certificate came to be accepted. The status line says "Trusted" only for
/// `Verified`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// The Mac's chain or a Trusted certificate.
    Verified,
    /// Trusted for this Session only, by choice or because remembering it failed.
    ThisSession,
    /// Trusted, and waiting for the Library to confirm it saved the Trusted certificate in the
    /// change with this ID.
    Remembering(ChangeId),
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
            waiting: None,
            menus_ready: false,
            next_change: 0,
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
        if !self.menus_ready {
            self.menus_ready = platform::release_hide_shortcuts();
        }
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
            Reply::Loaded(loaded) => {
                self.library = match loaded {
                    Loaded::Ready(snapshot) => Library::Ready(snapshot),
                    Loaded::Failed(e) => Library::Failed(e),
                };
                if let Some((address, username, password)) = self.waiting.take() {
                    self.connect(address, username, password);
                }
            }
            Reply::Saved(id, saved) => {
                let session = match &mut self.screen {
                    Screen::Session(live) if live.trust == Trust::Remembering(id) => Some(live),
                    _ => None,
                };
                match saved {
                    Ok(snapshot) => {
                        if let Some(live) = session {
                            live.trust = Trust::Verified;
                        }
                        self.library = Library::Ready(snapshot);
                    }
                    Err(e) => {
                        self.notice = Some(match session {
                            Some(live) => {
                                live.trust = Trust::ThisSession;
                                format!("The certificate is trusted for this session but was not remembered. {e}")
                            }
                            None => format!("A certificate was not remembered. {e}"),
                        });
                    }
                }
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
        self.waiting = None;
        self.screen = Screen::Idle;
    }

    /// A sign-in is queued until the Library loads.
    pub fn is_waiting(&self) -> bool {
        self.waiting.is_some()
    }

    pub fn sign_in(&mut self) {
        let Screen::SignIn(sign_in) = &mut self.screen else {
            return;
        };
        if self.waiting.is_some() {
            return;
        }
        let username = match Username::parse(&sign_in.username.text) {
            Ok(username) => username,
            Err(e) => return sign_in.error = Some(e.to_string()),
        };
        let address = sign_in.address.clone();
        let password = sign_in.password.text.clone();
        sign_in.password.clear();
        self.last_username = username.to_string();
        match self.library {
            Library::Loading => self.waiting = Some((address, username, password)),
            Library::Ready(_) | Library::Failed(_) => self.connect(address, username, password),
        }
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
            trust: Trust::Verified,
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
        let Some((_, prompt)) = self.prompt() else { return };
        let (host, port) = (prompt.request.host.clone(), prompt.request.port);
        // The fingerprint the person saw trusted for this host, so the save is refused if the
        // file holds another by then.
        let (remember, replacing) = match &prompt.request.question {
            Question::FirstUse(certificate) => (prompt.remember.then_some(certificate), None),
            Question::Changed { trusted, presented } => (Some(presented), Some(trusted.sha256)),
        };
        let change = remember.map(|c| Change::RememberCertificate {
            certificate: TrustedCertificate::new(&host, port, c, today()),
            replacing,
        });
        prompt.request.trust(change.is_some());
        let trust = match (change, &self.library, &self.store) {
            (None, ..) => Trust::ThisSession,
            (Some(change), Library::Ready(_), Some(store)) => {
                self.next_change += 1;
                store.apply(self.next_change, change);
                Trust::Remembering(self.next_change)
            }
            (Some(_), Library::Failed(e), _) => {
                self.notice = Some(format!(
                    "The certificate is trusted for this session but was not remembered. {e}"
                ));
                Trust::ThisSession
            }
            (Some(_), ..) => {
                self.notice = Some("The certificate is trusted for this session but was not remembered.".into());
                Trust::ThisSession
            }
        };
        if let Screen::Session(live) = &mut self.screen {
            live.trust = trust;
        }
    }

    pub fn dismiss_notice(&mut self) {
        self.notice = None;
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
        match keymap::translate(&mut live.keyboard, event) {
            Translated::Key(KeyOutcome::Send(ops)) => {
                if !ops.is_empty() {
                    live.handle.input(Input::Keys(ops));
                }
                true
            }
            Translated::Key(KeyOutcome::Shortcut(Shortcut::Close)) => {
                self.disconnect();
                true
            }
            // Tabs, the palette and Settings arrive in later milestones; until then Rune and
            // the menus handle these keys (⌘Q quits).
            Translated::Key(KeyOutcome::Shortcut(_)) | Translated::Pass => false,
            Translated::FocusLost => {
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
        trust: Trust::Verified,
    }
}

fn today() -> time::Date {
    time::OffsetDateTime::now_utc().date()
}
