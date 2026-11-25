//! The Main window's state and what happens to it: opening Connections and New hosts, signing
//! in, the Session's events, the certificate dialogs and the wrong-password prompt. Managing the
//! Library (sheets, Groups, Credentials, dragging) is in `manage`.

use std::sync::mpsc;

use bezel_core::address::Address;
use bezel_core::keys::{KeyOutcome, Keyboard, Shortcut};
use bezel_core::library::{Appearance, Change, Connection, ConnectionId, Credential, CredentialId, Snapshot};
use bezel_core::secrets::Secret;
use bezel_core::trust::{Question, TrustedCertificate};
use bezel_core::usernames::Username;
use bezel_rdp::{Auth, CertificateRequest, Ended, Event, Input, Pointer, Session, SessionHandle, State, Target, View};
use rune::{Key, KeyEvent, PhysicalInput, TextEdit, Tx, Waker};

use crate::keychain::Keychain;
use crate::keymap::{self, Translated};
use crate::manage::{Delete, Dragging, Sheet};
use crate::platform;
use crate::store::{Loaded, Reply, RequestId, Store};

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
    pub sidebar: bool,
    pub sheet: Option<Sheet>,
    /// A destructive choice waiting for confirmation, over any sheet.
    pub deleting: Option<Delete>,
    pub dragging: Option<Dragging>,
    next_session: u64,
    last_username: String,
    /// A sign-in waiting for the Library to load, so its Trusted certificate is known before
    /// the Session decides whether a certificate is new or changed.
    waiting: Option<Connect>,
    /// The app menu's Hide shortcuts no longer claim ⌘H and ⌥⌘H.
    menus_ready: bool,
    next_request: RequestId,
}

pub enum Library {
    Loading,
    Ready(Snapshot),
    Failed(String),
}

impl Library {
    pub fn ready(&self) -> Option<&Snapshot> {
        match self {
            Library::Ready(snapshot) => Some(snapshot),
            Library::Loading | Library::Failed(_) => None,
        }
    }
}

enum Message {
    Session(u64, Event),
    Store(Reply),
}

/// What a Session is for: a Connection, or a New host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    pub address: Address,
    pub connection: Option<ConnectionId>,
    /// The Connection's name, or the host of a New host.
    pub name: String,
}

impl Destination {
    pub fn new_host(address: Address) -> Self {
        Self {
            name: address.host.clone(),
            address,
            connection: None,
        }
    }

    pub fn connection(connection: &Connection) -> Self {
        Self {
            address: connection.address(),
            connection: Some(connection.id.clone()),
            name: connection.name.clone(),
        }
    }
}

pub enum Screen {
    Idle,
    SignIn(SignIn),
    /// Reading a saved Credential's password before connecting.
    Unlocking(Unlocking),
    Session(Live),
    /// The saved Credential's password was rejected, or none is stored.
    Retry(Retry),
}

pub struct SignIn {
    pub to: Destination,
    /// A saved Credential picked instead of typing a username and password.
    pub credential: Option<CredentialId>,
    pub username: TextEdit,
    pub password: TextEdit,
    /// "Save as Credential", and the name to save it under.
    pub save: bool,
    pub save_name: TextEdit,
    pub error: Option<String>,
}

impl SignIn {
    fn new(to: Destination, username: String, error: Option<String>) -> Self {
        Self {
            to,
            credential: None,
            username: TextEdit::new(username),
            password: TextEdit::default().secure(),
            save: false,
            save_name: TextEdit::default(),
            error,
        }
    }
}

pub struct Unlocking {
    pub to: Destination,
    pub credential: CredentialId,
    request: RequestId,
}

pub struct Retry {
    pub to: Destination,
    /// As the Library had it when the prompt opened, so the new password is saved only while
    /// the Credential still names the same account.
    pub credential: Credential,
    /// Assign the Credential to the Connection once signed in (see `Signing::assign`).
    assign: bool,
    /// The server rejected the password; otherwise the Keychain had none.
    pub rejected: bool,
    pub password: TextEdit,
    pub error: Option<String>,
    /// The save of the new password, and the password to retry with once it is saved.
    saving: Option<(RequestId, Secret)>,
}

impl Retry {
    pub fn saving(&self) -> bool {
        self.saving.is_some()
    }
}

pub struct Live {
    id: u64,
    pub to: Destination,
    pub username: String,
    /// The saved Credential it signs in with, so a rejected password asks for a new one.
    signing: Option<Signing>,
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
    Remembering(RequestId),
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

/// A sign-in ready to connect.
struct Connect {
    to: Destination,
    username: Username,
    password: Secret,
    signing: Option<Signing>,
}

/// The saved Credential a Session signs in with.
#[derive(Clone)]
struct Signing {
    /// As it was read for this sign-in.
    credential: Credential,
    /// The save of a Credential made from the sign-in sheet for this sign-in, still in flight.
    adding: Option<RequestId>,
    /// Assign the Credential to the Connection once signed in: it was saved from the sign-in
    /// sheet of a Connection that asks each time, so it stops asking.
    assign: bool,
}

/// A Connection's status dot in the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dot {
    Connected,
    Connecting,
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
            sidebar: true,
            sheet: None,
            deleting: None,
            dragging: None,
            next_session: 0,
            last_username: String::new(),
            waiting: None,
            menus_ready: false,
            next_request: 0,
        }
    }

    pub fn setup(&mut self, waker: Waker) {
        // A fixture scene brings its own Library; the person's file stays out of it.
        if matches!(self.library, Library::Loading) {
            let outbox = self.outbox.clone();
            self.store = Some(Store::open(
                platform::library_file(),
                Keychain::system(),
                move |reply| drop(outbox.send(Message::Store(reply))),
                waker.clone(),
            ));
        }
        self.waker = Some(waker);
    }

    /// Runs before every frame; true when something changed.
    pub fn poll(&mut self, _clock: f64, _tx: &mut Tx) -> bool {
        if !self.menus_ready {
            self.menus_ready = platform::release_hide_shortcuts();
        }
        self.drain()
    }

    /// Takes every Store reply and Session event that has arrived; true when there were any.
    fn drain(&mut self) -> bool {
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

    /// Light or dark from the Library's Appearance setting, else the system's.
    pub fn dark(&self, system: bool) -> bool {
        match self.library.ready().map(|s| s.settings.appearance) {
            Some(Appearance::Light) => false,
            Some(Appearance::Dark) => true,
            Some(Appearance::System) | None => system,
        }
    }

    fn on_store(&mut self, reply: Reply) {
        match reply {
            Reply::Loaded(loaded) => {
                self.library = match loaded {
                    Loaded::Ready(snapshot) => Library::Ready(snapshot),
                    Loaded::Failed(e) => Library::Failed(e),
                };
                if let Some(connect) = self.waiting.take() {
                    self.connect(connect);
                }
            }
            Reply::Saved { id, snapshot, error } => self.on_saved(id, snapshot, error),
            Reply::Password { id, password } => self.on_password(id, password),
        }
    }

    fn on_saved(&mut self, id: RequestId, snapshot: Snapshot, error: Option<String>) {
        // A refused change can still bring news from disk, such as a certificate another
        // Bezel remembered, which the next Session must expect.
        self.library = Library::Ready(snapshot);
        if let Screen::Session(live) = &mut self.screen
            && live.trust == Trust::Remembering(id)
        {
            live.trust = match error {
                None => Trust::Verified,
                Some(e) => {
                    self.notice = Some(format!(
                        "The certificate is trusted for this session but was not remembered. {e}"
                    ));
                    Trust::ThisSession
                }
            };
            return;
        }
        if let Screen::Session(live) = &mut self.screen
            && let Some(signing) = &mut live.signing
            && signing.adding == Some(id)
        {
            signing.adding = None;
            if let Some(e) = error {
                // Without its Credential the Session's password is a one-off again.
                live.signing = None;
                self.notice = Some(format!("The credential was not saved. {e}"));
            }
            return;
        }
        if let Screen::Retry(retry) = &mut self.screen
            && retry.saving.as_ref().is_some_and(|(saving, _)| *saving == id)
        {
            let (_, password) = retry.saving.take().expect("matched above");
            match error {
                None => {
                    let (to, credential, assign) = (retry.to.clone(), retry.credential.id.clone(), retry.assign);
                    self.connect_with(to, &credential, password, assign);
                }
                Some(e) => retry.error = Some(e),
            }
            return;
        }
        if self.on_sheet_saved(id, error.as_deref()) {
            return;
        }
        if let Some(e) = error {
            self.notice = Some(format!("Bezel couldn't save that change. {e}"));
        }
    }

    fn on_password(&mut self, id: RequestId, password: Result<Option<Secret>, String>) {
        let Screen::Unlocking(unlocking) = &self.screen else {
            return;
        };
        if unlocking.request != id {
            return;
        }
        let (to, credential) = (unlocking.to.clone(), unlocking.credential.clone());
        let error = match password {
            Ok(Some(password)) => return self.connect_with(to, &credential, password, false),
            Ok(None) => None,
            Err(e) => Some(format!("Bezel couldn't read the password from the Keychain. {e}")),
        };
        self.screen = match self.library.ready().and_then(|s| s.credential(&credential)) {
            Some(credential) => Screen::Retry(retry(to, credential.clone(), false, false, error)),
            None => Screen::SignIn(SignIn::new(to, self.last_username.clone(), Some(DELETED.into()))),
        };
    }

    /// Next ID for a store request.
    fn request(&mut self) -> RequestId {
        self.next_request += 1;
        self.next_request
    }

    /// Sends the change to the Library; `None`, with a notice, when the Library can't take
    /// changes because it is still loading or its file is broken.
    pub fn apply(&mut self, change: Change) -> Option<RequestId> {
        if let Library::Failed(e) = &self.library {
            self.notice = Some(format!("Bezel can't save changes until its data file is fixed. {e}"));
            return None;
        }
        if self.library.ready().is_none() {
            self.notice = Some("Bezel is still reading its data file, so that change was not saved.".into());
            return None;
        }
        let id = self.request();
        self.store.as_ref()?.apply(id, change);
        Some(id)
    }

    pub fn submit_host(&mut self) {
        match Address::parse(&self.host.text) {
            Ok(address) => {
                self.host_error = None;
                self.leave_session();
                let username = self.last_username.clone();
                self.screen = Screen::SignIn(SignIn::new(Destination::new_host(address), username, None));
            }
            Err(e) => self.host_error = Some(e.to_string()),
        }
    }

    /// Switches to the Connection's Session if it is the one open; otherwise ends the current
    /// one (one Session at a time until Tabs) and signs in to the Connection, silently when it
    /// has a Credential.
    pub fn open(&mut self, id: &ConnectionId) {
        let Some(snapshot) = self.library.ready() else { return };
        let Some(connection) = snapshot.connection(id) else {
            return;
        };
        if self.dot(id).is_some() {
            return;
        }
        let to = Destination::connection(connection);
        let credential = snapshot.credential_of(connection).map(|c| c.id.clone());
        self.leave_session();
        match credential {
            Some(credential) => self.unlock(to, credential),
            None => {
                let username = self.last_username.clone();
                self.screen = Screen::SignIn(SignIn::new(to, username, None));
            }
        }
    }

    /// The status dot for a Connection with a Session on its way or open.
    pub fn dot(&self, id: &ConnectionId) -> Option<Dot> {
        match &self.screen {
            Screen::Session(live) if live.to.connection.as_ref() == Some(id) => Some(match live.state {
                Phase::Connected { .. } => Dot::Connected,
                Phase::Connecting | Phase::Reconnecting => Dot::Connecting,
            }),
            Screen::Unlocking(unlocking) if unlocking.to.connection.as_ref() == Some(id) => Some(Dot::Connecting),
            _ => None,
        }
    }

    fn leave_session(&mut self) {
        if let Screen::Session(live) = &self.screen {
            live.handle.disconnect();
        }
        self.waiting = None;
        self.ended = None;
        self.screen = Screen::Idle;
    }

    fn unlock(&mut self, to: Destination, credential: CredentialId) {
        let request = self.request();
        if let Some(store) = &self.store {
            store.password(request, credential.clone());
        }
        self.screen = Screen::Unlocking(Unlocking {
            to,
            credential,
            request,
        });
    }

    /// Connects with a saved Credential's password, its username read from the Library now.
    fn connect_with(&mut self, to: Destination, credential: &CredentialId, password: Secret, assign: bool) {
        let Some(credential) = self.library.ready().and_then(|s| s.credential(credential)).cloned() else {
            self.screen = Screen::SignIn(SignIn::new(to, self.last_username.clone(), Some(DELETED.into())));
            return;
        };
        match Username::parse(&credential.username) {
            Ok(username) => self.connect(Connect {
                to,
                username,
                password,
                signing: Some(Signing {
                    credential,
                    adding: None,
                    assign,
                }),
            }),
            Err(e) => {
                let error = Some(format!("{} has an unusable username: {e}", credential.name));
                self.screen = Screen::SignIn(SignIn::new(to, self.last_username.clone(), error));
            }
        }
    }

    /// Makes the Connection sign in with the Credential from now on.
    fn assign(&mut self, connection: &ConnectionId, credential: CredentialId) {
        let Some(original) = self.library.ready().and_then(|s| s.connection(connection)).cloned() else {
            return;
        };
        let edited = Connection {
            credential: Some(credential),
            ..original.clone()
        };
        self.apply(Change::EditConnection { original, edited });
    }

    pub fn cancel_sign_in(&mut self) {
        self.waiting = None;
        self.screen = Screen::Idle;
    }

    /// A sign-in is queued until the Library loads.
    pub fn is_waiting(&self) -> bool {
        self.waiting.is_some()
    }

    /// Picks a saved Credential in the sign-in sheet, or `None` to type one.
    pub fn sign_in_with(&mut self, credential: Option<CredentialId>) {
        if let Screen::SignIn(sign_in) = &mut self.screen {
            sign_in.credential = credential;
            sign_in.error = None;
        }
    }

    pub fn toggle_save_credential(&mut self) {
        if let Screen::SignIn(sign_in) = &mut self.screen {
            sign_in.save = !sign_in.save;
            sign_in.save_name.focused = sign_in.save;
        }
    }

    pub fn sign_in(&mut self) {
        let Screen::SignIn(sign_in) = &mut self.screen else {
            return;
        };
        if self.waiting.is_some() {
            return;
        }
        if let Some(credential) = sign_in.credential.clone() {
            let to = sign_in.to.clone();
            return self.unlock(to, credential);
        }
        let username = match Username::parse(&sign_in.username.text) {
            Ok(username) => username,
            Err(e) => return sign_in.error = Some(e.to_string()),
        };
        let save_name = sign_in.save_name.text.trim().to_owned();
        if sign_in.save && save_name.is_empty() {
            return sign_in.error = Some("Enter a name for the credential".into());
        }
        let to = sign_in.to.clone();
        let password = Secret::from(sign_in.password.text.clone());
        sign_in.password.clear();
        let save = sign_in.save;
        self.last_username = username.to_string();
        // Saved as a Credential, a rejected password asks for a new one for that Credential.
        // Otherwise the typed password is the Session's own, and this sheet comes back.
        let signing = if save {
            let credential = Credential {
                id: CredentialId::random(),
                name: save_name,
                username: username.to_string(),
                password_updated: Some(today()),
            };
            let change = Change::AddCredential {
                credential: credential.clone(),
                password: password.clone(),
            };
            self.apply(change).map(|adding| Signing {
                credential,
                adding: Some(adding),
                assign: to.connection.is_some(),
            })
        } else {
            None
        };
        let connect = Connect {
            to,
            username,
            password,
            signing,
        };
        match self.library {
            Library::Loading => self.waiting = Some(connect),
            Library::Ready(_) | Library::Failed(_) => self.connect(connect),
        }
    }

    fn connect(&mut self, connect: Connect) {
        let Connect {
            to,
            username,
            password,
            signing,
        } = connect;
        let (trusted_certificate, command_key) = match &self.library {
            Library::Ready(snapshot) => (
                snapshot.trusted_certificate(&to.address.host, to.address.port).cloned(),
                snapshot.settings.command_key,
            ),
            Library::Loading | Library::Failed(_) => (None, Default::default()),
        };
        self.next_session += 1;
        let id = self.next_session;
        let (outbox, waker) = (self.outbox.clone(), self.waker.clone());
        let target = Target {
            host: to.address.host.clone(),
            port: to.address.port,
            view: self.area.unwrap_or(FALLBACK_VIEW),
            trusted_certificate,
        };
        let display_name = username.to_string();
        let handle = Session::connect(
            target,
            Auth {
                username,
                password: password.into_string().into(),
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
            to,
            username: display_name,
            signing,
            handle,
            keyboard: Keyboard::new(command_key),
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
            Event::State(State::Connected { width, height }) => {
                live.state = Phase::Connected { width, height };
                if let (Some(signing), Some(connection)) = (&mut live.signing, &live.to.connection)
                    && signing.assign
                {
                    signing.assign = false;
                    let (connection, credential) = (connection.clone(), signing.credential.id.clone());
                    self.assign(&connection, credential);
                }
            }
            Event::State(State::Reconnecting) => live.state = Phase::Reconnecting,
            Event::State(State::Disconnected(ended)) => {
                if !ended.by_user {
                    self.ended = Some(ended);
                }
                self.screen = Screen::Idle;
            }
            Event::Pointer(pointer) => live.pointer = pointer,
            Event::Certificate(request) => {
                let remember = self.library.ready().is_some_and(|s| s.settings.remember_certificates);
                live.certificate = Some(Prompt {
                    request,
                    remember,
                    confirming: false,
                })
            }
            Event::SignInFailed { wrong_password, ended } => {
                let to = live.to.clone();
                self.screen = match live.signing.clone() {
                    Some(signing) if wrong_password => {
                        Screen::Retry(retry(to, signing.credential, signing.assign, true, None))
                    }
                    Some(_) => {
                        self.ended = Some(ended);
                        Screen::Idle
                    }
                    None => Screen::SignIn(SignIn::new(to, self.last_username.clone(), Some(ended.message))),
                };
            }
        }
    }

    /// "Save and retry" on the wrong-password prompt: the password is saved to the Credential,
    /// for every Connection that uses it, and the Connection signs in again once it is.
    pub fn retry(&mut self) {
        let Screen::Retry(retry) = &mut self.screen else {
            return;
        };
        if retry.saving.is_some() || retry.password.text.is_empty() {
            return;
        }
        let password = Secret::from(retry.password.text.clone());
        let change = Change::EditCredential {
            original: retry.credential.clone(),
            edited: retry.credential.clone(),
            password: Some(password.clone()),
            today: today(),
        };
        retry.password.clear();
        retry.error = None;
        if let Some(id) = self.apply(change)
            && let Screen::Retry(retry) = &mut self.screen
        {
            retry.saving = Some((id, password));
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
        let trust = match change {
            None => Trust::ThisSession,
            Some(change) => match self.apply(change) {
                Some(id) => Trust::Remembering(id),
                None => {
                    let why = match &self.library {
                        Library::Failed(e) => format!(" {e}"),
                        Library::Loading | Library::Ready(_) => String::new(),
                    };
                    self.notice = Some(format!(
                        "The certificate is trusted for this session but was not remembered.{why}"
                    ));
                    Trust::ThisSession
                }
            },
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

    pub fn toggle_sidebar(&mut self) {
        self.sidebar = !self.sidebar;
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

    /// Keys pressed over the desktop. True when Bezel or the Session took the key.
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
            Translated::Key(KeyOutcome::Shortcut(Shortcut::Sidebar)) => {
                self.toggle_sidebar();
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

    /// Keys anywhere in the window that no control took: ⌃⌘S toggles the sidebar.
    pub fn on_key(&mut self, event: &KeyEvent, _tx: &mut Tx) -> bool {
        // With ⌃ held the key may arrive as its control character (⌃S is U+0013).
        let s = matches!(event.key, Key::Command('s' | '\u{13}'));
        if s && event.mods.ctrl && !event.mods.shift && !event.mods.alt {
            self.toggle_sidebar();
            return true;
        }
        false
    }
}

/// Shown on the sign-in sheet when the Credential a sign-in needs is gone.
const DELETED: &str = "The credential was deleted. Sign in another way.";

fn retry(to: Destination, credential: Credential, assign: bool, rejected: bool, error: Option<String>) -> Retry {
    let mut password = TextEdit::default().secure();
    password.focused = true;
    Retry {
        to,
        credential,
        assign,
        rejected,
        password,
        error,
        saving: None,
    }
}

/// For fixture scenes: a Session with nothing behind it.
pub fn fixture_session(to: Destination, state: Phase, certificate: Option<CertificateRequest>) -> Live {
    Live {
        id: 0,
        to,
        username: r"CORP\admin.max".into(),
        signing: None,
        handle: SessionHandle::detached(),
        keyboard: Keyboard::default(),
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

/// For fixture scenes: the wrong-password prompt.
pub fn fixture_retry(to: Destination, credential: Credential) -> Retry {
    retry(to, credential, false, true, None)
}

pub fn today() -> time::Date {
    time::OffsetDateTime::now_utc().date()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use bezel_core::library::{Group, GroupId, Library as Store_};
    use bezel_core::secrets::MemorySecrets;
    use rune::{Env, Runtime};

    use super::*;
    use crate::manage::Delete;

    /// A Bezel whose Library is a file in a temporary folder, holding the Groups Infrastructure
    /// and File servers, the Credential Domain admin (password "old") used by dc01, and the
    /// ungrouped lab, which asks each time. Hosts point at a local port that takes connections
    /// and never answers, so a Session stays connecting until the test says otherwise.
    struct Fixture {
        bezel: Bezel,
        secrets: MemorySecrets,
        _alive: (tempfile::TempDir, std::net::TcpListener),
    }

    fn fixture() -> Fixture {
        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = silent.local_addr().unwrap().port();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bezel.toml");
        let secrets = MemorySecrets::default();
        let mut library = Store_::open(&path, secrets.clone()).unwrap();
        let connection = |id: &str, name: &str, credential: Option<&str>, group: Option<&str>| Connection {
            id: id.into(),
            name: name.into(),
            host: "127.0.0.1".into(),
            port,
            credential: credential.map(Into::into),
            group: group.map(Into::into),
        };
        for change in [
            Change::AddGroup(Group {
                id: "g_infra".into(),
                name: "Infrastructure".into(),
            }),
            Change::AddGroup(Group {
                id: "g_files".into(),
                name: "File servers".into(),
            }),
            Change::AddCredential {
                credential: Credential {
                    id: "c_admin".into(),
                    name: "Domain admin".into(),
                    username: r"CORP\admin.max".into(),
                    password_updated: None,
                },
                password: Secret::from("old".to_owned()),
            },
            Change::AddConnection(connection("n_dc01", "dc01", Some("c_admin"), Some("g_infra"))),
            Change::AddConnection(connection("n_lab", "lab", None, None)),
        ] {
            library.apply(change).unwrap();
        }
        let mut bezel = Bezel::new();
        let outbox = bezel.outbox.clone();
        bezel.store = Some(Store::open(
            path,
            secrets.clone(),
            move |reply| drop(outbox.send(Message::Store(reply))),
            Arc::new(|| {}),
        ));
        wait_until(&mut bezel, |b| b.library.ready().is_some());
        Fixture {
            bezel,
            secrets,
            _alive: (dir, silent),
        }
    }

    fn wait_until(bezel: &mut Bezel, done: impl Fn(&Bezel) -> bool) {
        for _ in 0..500 {
            bezel.drain();
            if done(bezel) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out");
    }

    fn snapshot(bezel: &Bezel) -> &Snapshot {
        bezel.library.ready().unwrap()
    }

    fn rejected() -> Event {
        Event::SignInFailed {
            wrong_password: true,
            ended: Ended {
                message: "The username or password is incorrect".into(),
                details: String::new(),
                by_user: false,
            },
        }
    }

    /// dc01's Session as if it had signed in with its Credential.
    fn dc01_session(bezel: &Bezel, credential: Option<&str>) -> Live {
        let dc01 = snapshot(bezel).connection(&"n_dc01".into()).unwrap();
        let signing = credential.map(|id| Signing {
            credential: snapshot(bezel).credential(&id.into()).unwrap().clone(),
            adding: None,
            assign: false,
        });
        Live {
            id: 7,
            signing,
            ..fixture_session(Destination::connection(dc01), Phase::Connecting, None)
        }
    }

    /// Feeds the open Session an event from its engine.
    fn session_event(bezel: &mut Bezel, event: Event) {
        let Screen::Session(live) = &bezel.screen else {
            panic!("no Session");
        };
        bezel.on_session(live.id, event);
    }

    fn connected() -> Event {
        Event::State(State::Connected {
            width: 2048,
            height: 1280,
        })
    }

    #[test]
    fn a_rejected_password_is_saved_to_the_credential_and_the_connection_retries() {
        let Fixture {
            mut bezel,
            secrets,
            _alive,
        } = fixture();
        bezel.screen = Screen::Session(dc01_session(&bezel, Some("c_admin")));
        bezel.on_session(7, rejected());
        let Screen::Retry(retry) = &mut bezel.screen else {
            panic!("expected the wrong-password prompt");
        };
        assert!(retry.rejected);
        assert_eq!(retry.credential.id, CredentialId::from("c_admin"));
        retry.password = TextEdit::new("new");
        bezel.retry();
        assert!(matches!(&bezel.screen, Screen::Retry(r) if r.saving()));
        let sessions = bezel.next_session;
        wait_until(&mut bezel, |b| b.next_session > sessions);
        assert_eq!(secrets.get(&"c_admin".into()), Some(Secret::from("new".to_owned())));
        assert_eq!(
            snapshot(&bezel).credential(&"c_admin".into()).unwrap().password_updated,
            Some(today())
        );
        // The retry is dc01 signing in with the saved Credential again.
        if let Screen::Session(live) = &bezel.screen {
            assert_eq!(live.to.connection, Some("n_dc01".into()));
            assert_eq!(
                live.signing.as_ref().map(|s| s.credential.id.clone()),
                Some("c_admin".into())
            );
        }
    }

    #[test]
    fn a_rejected_typed_password_brings_the_sign_in_sheet_back() {
        let Fixture { mut bezel, _alive, .. } = fixture();
        bezel.screen = Screen::Session(dc01_session(&bezel, None));
        bezel.on_session(7, rejected());
        assert!(matches!(&bezel.screen, Screen::SignIn(s) if s.error.is_some() && s.to.name == "dc01"));
    }

    #[test]
    fn a_connection_with_a_credential_reads_its_password_and_connects() {
        let Fixture { mut bezel, _alive, .. } = fixture();
        bezel.open(&"n_dc01".into());
        assert_eq!(bezel.dot(&"n_dc01".into()), Some(Dot::Connecting));
        wait_until(&mut bezel, |b| b.next_session == 1);
        let lab = ConnectionId::from("n_lab");
        bezel.open(&lab);
        assert!(matches!(&bezel.screen, Screen::SignIn(s) if s.to.connection == Some(lab.clone())));
    }

    #[test]
    fn a_missing_password_asks_for_one_and_stale_replies_are_ignored() {
        let Fixture { mut bezel, _alive, .. } = fixture();
        let to = Destination::connection(snapshot(&bezel).connection(&"n_dc01".into()).unwrap());
        bezel.screen = Screen::Unlocking(Unlocking {
            to,
            credential: "c_admin".into(),
            request: 5,
        });
        bezel.on_password(4, Ok(Some(Secret::from("stale".to_owned()))));
        assert!(matches!(bezel.screen, Screen::Unlocking(_)));
        bezel.on_password(5, Ok(None));
        assert!(matches!(&bezel.screen, Screen::Retry(r) if !r.rejected));
    }

    /// Signs in to lab, which asks each time, with Save as credential ticked.
    fn sign_in_saving(bezel: &mut Bezel, password: &str) -> Credential {
        bezel.open(&"n_lab".into());
        let Screen::SignIn(sign_in) = &mut bezel.screen else {
            panic!()
        };
        sign_in.username = TextEdit::new("administrator");
        sign_in.password = TextEdit::new(password);
        bezel.toggle_save_credential();
        bezel.sign_in();
        assert!(
            matches!(&bezel.screen, Screen::SignIn(s) if s.error.is_some()),
            "a name is needed"
        );
        let Screen::SignIn(sign_in) = &mut bezel.screen else {
            panic!()
        };
        sign_in.password = TextEdit::new(password);
        sign_in.save_name = TextEdit::new("Local admin");
        bezel.sign_in();
        wait_until(bezel, |b| snapshot(b).credentials.len() == 2);
        snapshot(bezel).credentials[1].clone()
    }

    #[test]
    fn a_credential_saved_at_sign_in_becomes_the_connections_once_signed_in() {
        let Fixture {
            mut bezel,
            secrets,
            _alive,
        } = fixture();
        let saved = sign_in_saving(&mut bezel, "typed");
        assert_eq!(
            (saved.name.as_str(), saved.username.as_str()),
            ("Local admin", "administrator")
        );
        assert_eq!(secrets.get(&saved.id), Some(Secret::from("typed".to_owned())));
        let lab = ConnectionId::from("n_lab");
        assert_eq!(snapshot(&bezel).connection(&lab).unwrap().credential, None);
        session_event(&mut bezel, connected());
        wait_until(&mut bezel, |b| {
            snapshot(b).connection(&lab).unwrap().credential == Some(saved.id.clone())
        });
    }

    #[test]
    fn a_rejected_password_saved_at_sign_in_is_corrected_in_that_credential() {
        let Fixture {
            mut bezel,
            secrets,
            _alive,
        } = fixture();
        let saved = sign_in_saving(&mut bezel, "typo");
        session_event(&mut bezel, rejected());
        let Screen::Retry(retry) = &mut bezel.screen else {
            panic!("expected the wrong-password prompt for the new Credential");
        };
        assert_eq!(retry.credential.id, saved.id);
        retry.password = TextEdit::new("right");
        bezel.retry();
        let sessions = bezel.next_session;
        wait_until(&mut bezel, |b| b.next_session > sessions);
        assert_eq!(secrets.get(&saved.id), Some(Secret::from("right".to_owned())));
        session_event(&mut bezel, connected());
        wait_until(&mut bezel, |b| {
            snapshot(b).connection(&"n_lab".into()).unwrap().credential == Some(saved.id.clone())
        });
    }

    /// Rune refuses to show a menu with an empty or repeated item ID.
    fn assert_shows(menu: &rune::PopupMenu) {
        fn ids(menu: &rune::PopupMenu, out: &mut Vec<String>) {
            for entry in menu.entries() {
                match entry {
                    rune::PopupEntry::Item(item) => out.push(item.id().to_owned()),
                    rune::PopupEntry::Submenu { menu, .. } => ids(menu, out),
                    _ => {}
                }
            }
        }
        let mut all = Vec::new();
        ids(menu, &mut all);
        let unique: std::collections::HashSet<&String> = all.iter().collect();
        assert!(
            all.iter().all(|id| !id.is_empty()) && unique.len() == all.len(),
            "{all:?}"
        );
    }

    fn press(rt: &mut Runtime<Bezel>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no {key}"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
    }

    /// Opens the picker and answers its menu with the item titled `title`.
    fn pick(rt: &mut Runtime<Bezel>, key: &str, title: &str) {
        press(rt, &format!("{key}-trigger"));
        let menu = rt.popup_menu().expect("the picker asked for a menu");
        assert_shows(menu);
        let id = menu
            .entries()
            .iter()
            .find_map(|entry| match entry {
                rune::PopupEntry::Item(item) if item.title() == title => Some(item.id().to_owned()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {title}"));
        rt.resolve_popup(Some(&id)).unwrap();
        rt.frame(1.0 / 60.0);
    }

    #[test]
    fn pickers_offer_menus_rune_shows_and_set_what_is_picked() {
        let Fixture { mut bezel, _alive, .. } = fixture();
        bezel.edit_connection(&"n_dc01".into());
        let mut rt = Runtime::new(bezel, crate::view::view, (1024.0, 700.0), Env::new(false, 1.0));
        rt.frame(0.0);
        pick(&mut rt, "connection-group", "File servers");
        pick(&mut rt, "connection-credential", "Ask each time");
        let Some(Sheet::Connection(form)) = &rt.state.sheet else {
            panic!()
        };
        assert_eq!((&form.group, &form.credential), (&Some("g_files".into()), &None));
        pick(&mut rt, "connection-group", "None");
        pick(&mut rt, "connection-credential", "Domain admin");
        let Some(Sheet::Connection(form)) = &rt.state.sheet else {
            panic!()
        };
        assert_eq!((&form.group, &form.credential), (&None, &Some("c_admin".into())));
    }

    #[test]
    fn context_menus_offer_menus_rune_shows() {
        let Fixture { bezel, _alive, .. } = fixture();
        let mut rt = Runtime::new(bezel, crate::view::view, (1024.0, 700.0), Env::new(false, 1.0));
        rt.frame(0.0);
        for key in ["n_dc01", "section-g_infra/section-header"] {
            let (x, y) = rt.find(key).unwrap();
            rt.secondary_down(x, y);
            rt.secondary_up(x, y);
            rt.frame(1.0 / 60.0);
            assert_shows(rt.popup_menu().expect("a context menu"));
            rt.resolve_popup(None).unwrap();
            rt.frame(1.0 / 60.0);
        }
        let (x, y) = rt.find("n_lab").unwrap();
        rt.secondary_down(x, y);
        rt.secondary_up(x, y);
        rt.frame(1.0 / 60.0);
        let move_to_files = rt
            .popup_menu()
            .and_then(|menu| {
                menu.entries().iter().find_map(|entry| match entry {
                    rune::PopupEntry::Submenu { menu, .. } => menu.entries().iter().find_map(|e| match e {
                        rune::PopupEntry::Item(item) if item.title() == "File servers" => Some(item.id().to_owned()),
                        _ => None,
                    }),
                    _ => None,
                })
            })
            .unwrap();
        rt.resolve_popup(Some(&move_to_files)).unwrap();
        rt.frame(1.0 / 60.0);
        settle(&mut rt, |b| {
            snapshot(b).connection(&"n_lab".into()).unwrap().group == Some("g_files".into())
        });
    }

    #[test]
    fn the_connection_sheet_adds_and_keeps_its_input_when_a_save_is_refused() {
        let Fixture { mut bezel, _alive, .. } = fixture();
        bezel.new_connection(Some("g_files".into()));
        if let Some(Sheet::Connection(form)) = &mut bezel.sheet {
            form.host = TextEdit::new("fs01.corp.lan:3390");
        }
        bezel.save_connection();
        wait_until(&mut bezel, |b| b.sheet.is_none());
        let fs01 = snapshot(&bezel).connections.iter().find(|c| c.name == "fs01").unwrap();
        assert_eq!((fs01.port, &fs01.group), (3390, &Some(GroupId::from("g_files"))));

        bezel.edit_connection(&"n_lab".into());
        bezel.apply(Change::DeleteConnection("n_lab".into()));
        bezel.save_connection();
        wait_until(
            &mut bezel,
            |b| matches!(&b.sheet, Some(Sheet::Connection(f)) if f.error.is_some()),
        );
    }

    #[test]
    fn deleting_a_credential_makes_its_connections_ask() {
        let Fixture {
            mut bezel,
            secrets,
            _alive,
        } = fixture();
        bezel.open_credentials(None);
        bezel.ask_delete(Delete::Credential("c_admin".into()));
        bezel.delete();
        wait_until(&mut bezel, |b| snapshot(b).credentials.is_empty());
        assert_eq!(snapshot(&bezel).connection(&"n_dc01".into()).unwrap().credential, None);
        assert_eq!(secrets.get(&"c_admin".into()), None);
    }

    #[test]
    fn the_credentials_sheet_changes_a_password_for_every_connection() {
        let Fixture {
            mut bezel,
            secrets,
            _alive,
        } = fixture();
        bezel.open_credentials(None);
        if let Some(Sheet::Credentials(sheet)) = &mut bezel.sheet {
            sheet.password = TextEdit::new("changed");
        }
        bezel.save_credential();
        wait_until(
            &mut bezel,
            |b| matches!(&b.sheet, Some(Sheet::Credentials(s)) if !s.saving()),
        );
        assert_eq!(secrets.get(&"c_admin".into()), Some(Secret::from("changed".to_owned())));
    }

    fn drag(rt: &mut Runtime<Bezel>, from: &str, to: &str) {
        let (x, y) = rt.find(from).unwrap();
        let (tx, ty) = rt.find(to).unwrap();
        rt.pointer_down(x, y);
        for step in 1..=10 {
            let t = step as f32 / 10.0;
            rt.pointer_move(x + (tx - x) * t, y + (ty - y) * t);
            rt.frame(1.0 / 60.0);
        }
        rt.pointer_up(tx, ty);
        rt.frame(1.0 / 60.0);
    }

    /// Waits on the Store as the window's poll would, then draws what it brought.
    fn settle(rt: &mut Runtime<Bezel>, done: impl Fn(&Bezel) -> bool) {
        wait_until(&mut rt.state, done);
        rt.invalidate();
        rt.frame(1.0 / 60.0);
    }

    #[test]
    fn dragging_moves_connections_between_groups_and_reorders_groups() {
        let Fixture { bezel, _alive, .. } = fixture();
        let mut rt = Runtime::new(bezel, crate::view::view, (1024.0, 700.0), Env::new(false, 1.0));
        rt.frame(0.0);
        drag(&mut rt, "n_lab", "section-g_files");
        settle(&mut rt, |b| {
            snapshot(b).connection(&"n_lab".into()).unwrap().group == Some("g_files".into())
        });
        assert!(matches!(rt.state.screen, Screen::Idle), "a drag is not a click");
        drag(
            &mut rt,
            "section-g_files/section-header",
            "section-g_infra/section-header",
        );
        settle(&mut rt, |b| snapshot(b).groups[0].id == GroupId::from("g_files"));
        let (x, y) = rt.find("n_dc01/actions/edit").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
        assert!(
            matches!(&rt.state.sheet, Some(Sheet::Connection(f)) if f.editing.as_ref().is_some_and(|c| c.id.as_str() == "n_dc01"))
        );
        assert!(
            matches!(rt.state.screen, Screen::Idle),
            "the hover action is not a click on the row"
        );
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: Default::default(),
        });
        rt.frame(1.0 / 60.0);
        assert!(rt.state.sheet.is_none());
        // The sheet fades out before the sidebar takes clicks again.
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        let (x, y) = rt.find("n_lab").unwrap();
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
        assert!(matches!(&rt.state.screen, Screen::SignIn(s) if s.to.name == "lab"));
    }
}
