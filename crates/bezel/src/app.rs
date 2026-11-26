//! The app's state, which every window shares (see `windows`): the Library, the Sessions in
//! Tabs and Session windows, and what happens to them: opening Connections and New hosts,
//! signing in, each Session's events, the certificate dialogs and the wrong-password prompt.
//! Managing the Library (sheets, Groups, Credentials, dragging) is in `manage`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use bezel_core::address::Address;
use bezel_core::keys::{KeyOutcome, Keyboard, Shortcut};
use bezel_core::library::{Appearance, Change, Connection, ConnectionId, Credential, CredentialId, Snapshot};
use bezel_core::secrets::Secret;
use bezel_core::trust::{Question, TrustedCertificate};
use bezel_core::usernames::Username;
use bezel_rdp::{Auth, CertificateRequest, Ended, Event, Input, Pointer, SessionHandle, State, Target, View};
use rune::{AppHandle, Key, KeyEvent, PhysicalInput, TextEdit, Tx, Waker, WindowHandle};
use rune_kit::shell::TabDragState;

use crate::keychain::Keychain;
use crate::keymap::{self, Translated};
use crate::manage::{Delete, Dragging, Sheet};
use crate::platform;
use crate::store::{Loaded, Reply, RequestId, Store};
use crate::windows::{Alarm, Here, Home, Window};

pub struct Bezel {
    /// The window whose turn it is; each window keeps its own through every turn.
    pub here: Here,
    pub(crate) home: Option<Home>,
    pub(crate) alarm: Alarm,
    /// Counts changes another window may show, so each window redraws once it sees a new one.
    pub(crate) generation: u64,
    /// What other windows show of this state, as last seen (see `windows::outline`).
    pub(crate) outline: u64,
    inbox: mpsc::Receiver<Message>,
    outbox: mpsc::Sender<Message>,
    store: Option<Store>,
    pub library: Library,
    /// Every Session, in Tab order for those in Tabs.
    pub sessions: Vec<Session>,
    /// The Tab the Main window shows.
    pub active: Option<SessionId>,
    /// Each window's content area, as last laid out.
    pub(crate) areas: Vec<(Window, View)>,
    /// A problem saving the Library, shown until dismissed.
    pub notice: Option<String>,
    pub sidebar: bool,
    pub sheet: Option<Sheet>,
    /// A destructive choice waiting for confirmation, over any sheet.
    pub deleting: Option<Delete>,
    pub dragging: Option<Dragging>,
    pub tabs_drag: TabDragState,
    /// The Main window's tab row in its window points, for a Session window dropped on it.
    pub(crate) tab_row: Option<[f32; 4]>,
    /// ⌘Q is asking whether to disconnect the open Sessions.
    pub quitting: bool,
    pub(crate) main: Option<AppHandle<Bezel>>,
    /// The Main window's native window, once it exists.
    pub(crate) main_window: Rc<RefCell<Option<WindowHandle>>>,
    /// Stands in for a text field whose form has gone, so an edit lands nowhere.
    pub scratch: TextEdit,
    next_id: u64,
    pub(crate) next_window: u64,
    last_username: String,
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
    /// From the engine with this ID, which one Session's `Live` names.
    Engine(u64, Event),
    Store(Reply),
}

pub type SessionId = u64;

/// One Session in a Tab or a Session window, from signing in until it is closed.
pub struct Session {
    pub id: SessionId,
    pub to: Destination,
    pub place: Place,
    pub screen: Screen,
    /// A problem saving something for this Session, shown over it until dismissed.
    pub notice: Option<String>,
    /// The save making the Credential this Session signed in with its Connection's, in flight,
    /// and the Credential's name. Kept here so its reply finds the Session after it ends.
    assigning: Option<(RequestId, String)>,
}

pub enum Place {
    Tab,
    /// Its Session window, by the native window's key.
    Window(WindowHandle),
}

impl Place {
    pub fn is_tab(&self) -> bool {
        matches!(self, Place::Tab)
    }
}

/// Where to open a Connection or New host that isn't open yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opening {
    Tab,
    Window,
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
    SignIn(SignIn),
    /// Reading a saved Credential's password before connecting.
    Unlocking(Unlocking),
    Live(Live),
    /// The saved Credential's password was rejected, or none is stored.
    Retry(Retry),
    /// The Session ended without being asked to, and says why.
    Ended(Ended),
}

pub struct SignIn {
    /// A saved Credential picked instead of typing a username and password.
    pub credential: Option<CredentialId>,
    pub username: TextEdit,
    pub password: TextEdit,
    /// "Save as Credential", and the name to save it under.
    pub save: bool,
    pub save_name: TextEdit,
    pub error: Option<String>,
    /// The sign-in, waiting for the Library to load so its Trusted certificate is known before
    /// the Session decides whether a certificate is new or changed.
    waiting: Option<Connect>,
}

impl SignIn {
    fn new(username: String, error: Option<String>) -> Self {
        let mut username = TextEdit::new(username);
        username.focused = true;
        Self {
            credential: None,
            username,
            password: TextEdit::default().secure(),
            save: false,
            save_name: TextEdit::default(),
            error,
            waiting: None,
        }
    }

    /// A sign-in is queued until the Library loads.
    pub fn waiting(&self) -> bool {
        self.waiting.is_some()
    }
}

pub struct Unlocking {
    pub credential: CredentialId,
    request: RequestId,
}

pub struct Retry {
    /// As the Library had it when the prompt opened, so the new password is saved only while
    /// the Credential still names the same account.
    pub credential: Credential,
    /// Assign the Credential to the Connection once signed in (see `Signing::assign`).
    assign: bool,
    /// The Credential was saved from the sign-in sheet for the rejected sign-in, and this is
    /// that save, still in flight: the prompt waits for it, and falls back to the sign-in
    /// sheet if it is refused.
    adding: Option<RequestId>,
    /// The server rejected the password; otherwise the Keychain had none.
    pub rejected: bool,
    pub password: TextEdit,
    pub error: Option<String>,
    /// The save of the new password, with the password and the Credential it was typed for,
    /// to sign in with once it is saved.
    saving: Option<(RequestId, Secret, Credential)>,
}

impl Retry {
    pub fn saving(&self) -> bool {
        self.saving.is_some() || self.adding.is_some()
    }
}

pub struct Live {
    /// The engine this Session's events come from; a retry starts another.
    engine: u64,
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

impl Live {
    /// Forgets every key held, as when the desktop loses the keyboard.
    pub(crate) fn release_keys(&mut self) {
        self.keyboard.release_all();
    }
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

/// A Session's status dot, in its Tab and the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dot {
    Connected,
    Connecting,
}

impl Session {
    pub fn dot(&self) -> Option<Dot> {
        match &self.screen {
            Screen::Live(live) => Some(match live.state {
                Phase::Connected { .. } => Dot::Connected,
                Phase::Connecting | Phase::Reconnecting => Dot::Connecting,
            }),
            Screen::Unlocking(_) => Some(Dot::Connecting),
            Screen::SignIn(_) | Screen::Retry(_) | Screen::Ended(_) => None,
        }
    }

    /// Whether ending it would disconnect something: ⌘Q asks about these.
    pub fn is_open(&self) -> bool {
        matches!(self.screen, Screen::Live(_) | Screen::Unlocking(_))
    }

    pub fn live(&self) -> Option<&Live> {
        match &self.screen {
            Screen::Live(live) => Some(live),
            _ => None,
        }
    }

    /// Whether the Store reply with this ID is this Session's.
    fn awaits(&self, id: RequestId) -> bool {
        if self.assigning.as_ref().is_some_and(|(assigning, _)| *assigning == id) {
            return true;
        }
        match &self.screen {
            Screen::Live(live) => {
                live.trust == Trust::Remembering(id) || live.signing.as_ref().is_some_and(|s| s.adding == Some(id))
            }
            Screen::Retry(retry) => {
                retry.adding == Some(id) || retry.saving.as_ref().is_some_and(|(saving, ..)| *saving == id)
            }
            Screen::Unlocking(unlocking) => unlocking.request == id,
            Screen::SignIn(_) | Screen::Ended(_) => false,
        }
    }
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
            here: Here::main(),
            home: None,
            alarm: Alarm::default(),
            generation: 0,
            outline: 0,
            inbox,
            outbox,
            store: None,
            library: Library::Loading,
            sessions: Vec::new(),
            active: None,
            areas: Vec::new(),
            notice: None,
            sidebar: true,
            sheet: None,
            deleting: None,
            dragging: None,
            tabs_drag: TabDragState::new(),
            tab_row: None,
            quitting: false,
            main: None,
            main_window: Rc::default(),
            scratch: TextEdit::default(),
            next_id: 0,
            next_window: 0,
            last_username: String::new(),
            menus_ready: false,
            next_request: 0,
        }
    }

    pub fn setup(&mut self, waker: Waker) {
        self.alarm.set(&self.here.key, waker.clone());
        self.here.waker = Some(waker);
        // A window state that stands in for the shared one (see `windows`) opens nothing, and a
        // fixture scene brings its own Library; the person's file stays out of it.
        if self.home.is_none() && matches!(self.library, Library::Loading) {
            self.open_store();
        }
    }

    pub(crate) fn open_store(&mut self) {
        let (outbox, alarm) = (self.outbox.clone(), self.alarm.clone());
        self.store = Some(Store::open(
            platform::library_file(),
            Keychain::system(),
            move |reply| drop(outbox.send(Message::Store(reply))),
            std::sync::Arc::new(move || alarm.ring()),
        ));
    }

    /// Runs before every frame; true when something changed.
    pub fn poll(&mut self, _clock: f64, tx: &mut Tx) -> bool {
        if !self.menus_ready {
            self.menus_ready = platform::release_hide_shortcuts();
        }
        if self.drain() {
            self.changed();
        }
        self.title_window(tx);
        let fresh = self.here.seen != self.generation;
        self.here.seen = self.generation;
        fresh
    }

    /// Takes every Store reply and Session event that has arrived; true when there were any.
    fn drain(&mut self) -> bool {
        let mut changed = false;
        while let Ok(message) = self.inbox.try_recv() {
            changed = true;
            match message {
                Message::Store(reply) => self.on_store(reply),
                Message::Engine(engine, event) => self.on_engine(engine, event),
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

    pub fn session(&self, id: SessionId) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == id)
    }

    pub(crate) fn session_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    fn screen_mut(&mut self, id: SessionId) -> Option<&mut Screen> {
        self.session_mut(id).map(|s| &mut s.screen)
    }

    /// The Session this window shows: the Main window's Tab, or a Session window's own.
    pub fn current(&self) -> Option<SessionId> {
        match self.here.window {
            Window::Main => self.active,
            Window::Session(id) => Some(id),
        }
    }

    pub fn tabs(&self) -> impl Iterator<Item = &Session> {
        self.sessions.iter().filter(|s| s.place.is_tab())
    }

    /// A text field of a Session's screen, or the scratch field once that screen has gone.
    pub(crate) fn field(&mut self, id: SessionId, pick: fn(&mut Screen) -> Option<&mut TextEdit>) -> &mut TextEdit {
        let scratch = &mut self.scratch;
        match self
            .sessions
            .iter_mut()
            .find(|s| s.id == id)
            .and_then(|s| pick(&mut s.screen))
        {
            Some(edit) => edit,
            None => scratch,
        }
    }

    fn on_store(&mut self, reply: Reply) {
        match reply {
            Reply::Loaded(loaded) => {
                self.library = match loaded {
                    Loaded::Ready(snapshot) => Library::Ready(snapshot),
                    Loaded::Failed(e) => Library::Failed(e),
                };
                let waiting: Vec<(SessionId, Connect)> = self
                    .sessions
                    .iter_mut()
                    .filter_map(|s| match &mut s.screen {
                        Screen::SignIn(sign_in) => sign_in.waiting.take().map(|c| (s.id, c)),
                        _ => None,
                    })
                    .collect();
                for (id, connect) in waiting {
                    self.connect(id, connect);
                }
            }
            Reply::Saved { id, snapshot, error } => self.on_saved(id, snapshot, error),
            Reply::Password { id, password } => self.on_password(id, password),
        }
    }

    fn on_saved(&mut self, id: RequestId, snapshot: Snapshot, error: Option<String>) {
        // A refused change can still bring news from disk, such as a certificate another
        // Bezel remembered, which the next Session must expect, and every open form follows it.
        self.library = Library::Ready(snapshot);
        self.follow_library();
        if let Some(session) = self.sessions.iter().find(|s| s.awaits(id)).map(|s| s.id) {
            return self.on_session_saved(session, id, error);
        }
        if self.on_sheet_saved(id, error.as_deref()) {
            return;
        }
        if let Some(e) = error {
            self.notice = Some(format!("Bezel couldn't save that change. {e}"));
        }
    }

    /// The reply to a change the Session `session` made.
    fn on_session_saved(&mut self, session: SessionId, id: RequestId, error: Option<String>) {
        let Some(s) = self.session_mut(session) else { return };
        if let Some((_, credential)) = s.assigning.take_if(|(assigning, _)| *assigning == id) {
            if let Some(e) = error {
                s.notice = Some(format!(
                    "{credential} was not saved as this connection's credential, so it will ask again. {e}"
                ));
            }
            return;
        }
        match &mut s.screen {
            Screen::Live(live) if live.trust == Trust::Remembering(id) => {
                live.trust = match error {
                    None => Trust::Verified,
                    Some(e) => {
                        s.notice = Some(format!(
                            "The certificate is trusted for this session but was not remembered. {e}"
                        ));
                        Trust::ThisSession
                    }
                };
            }
            Screen::Live(live) => {
                let Some(signing) = &mut live.signing else { return };
                signing.adding = None;
                match error {
                    None => self.assign_when_signed_in(session),
                    Some(e) => {
                        // Without its Credential the Session's password is a one-off again.
                        live.signing = None;
                        s.notice = Some(format!("The credential was not saved. {e}"));
                    }
                }
            }
            Screen::Retry(retry) if retry.adding == Some(id) => {
                retry.adding = None;
                if let Some(e) = error {
                    let error = Some(format!("The credential was not saved, so sign in again. {e}"));
                    s.screen = Screen::SignIn(SignIn::new(retry.credential.username.clone(), error));
                }
            }
            Screen::Retry(retry) => {
                let Some((_, password, credential)) = retry.saving.take() else {
                    return;
                };
                match error {
                    None => {
                        let assign = retry.assign;
                        self.connect_as(session, credential, password, assign);
                    }
                    Some(e) => {
                        retry.error = Some(e);
                        // The save may have been refused because the Credential is gone, and
                        // then no retry can succeed.
                        self.follow_retry(session);
                    }
                }
            }
            Screen::Unlocking(_) | Screen::SignIn(_) | Screen::Ended(_) => {}
        }
    }

    fn on_password(&mut self, id: RequestId, password: Result<Option<Secret>, String>) {
        let Some(session) = self.sessions.iter().find(|s| s.awaits(id)) else {
            return;
        };
        let Screen::Unlocking(unlocking) = &session.screen else {
            return;
        };
        let session = session.id;
        let Some(credential) = self
            .library
            .ready()
            .and_then(|s| s.credential(&unlocking.credential))
            .cloned()
        else {
            return self.ask(session, Some(DELETED.into()));
        };
        let error = match password {
            Ok(Some(password)) => return self.connect_as(session, credential, password, false),
            Ok(None) => None,
            Err(e) => Some(format!("Bezel couldn't read the password from the Keychain. {e}")),
        };
        if let Some(screen) = self.screen_mut(session) {
            *screen = Screen::Retry(retry(credential, false, false, error));
        }
    }

    /// Shows the sign-in sheet for the Session.
    fn ask(&mut self, session: SessionId, error: Option<String>) {
        let username = self.last_username.clone();
        if let Some(screen) = self.screen_mut(session) {
            *screen = Screen::SignIn(SignIn::new(username, error));
        }
    }

    /// Keeps every wrong-password prompt bound to the account it asks about.
    pub(crate) fn follow_retries(&mut self) {
        let ids: Vec<SessionId> = self.sessions.iter().map(|s| s.id).collect();
        for id in ids {
            self.follow_retry(id);
        }
    }

    /// When the Credential's username changes, the password typed for the old one is cleared,
    /// and when the Credential is gone the sign-in sheet takes over.
    fn follow_retry(&mut self, session: SessionId) {
        let Some(snapshot) = self.library.ready() else { return };
        let Some(Session {
            screen: Screen::Retry(retry),
            ..
        }) = self.sessions.iter_mut().find(|s| s.id == session)
        else {
            return;
        };
        match snapshot.credential(&retry.credential.id) {
            Some(now) => {
                if now.username != retry.credential.username {
                    retry.password = TextEdit::default().secure();
                    retry.password.focused = true;
                    retry.error = Some(format!(
                        "The username of {} changed to {}. Enter the password for {}.",
                        now.name, now.username, now.username
                    ));
                }
                retry.credential = now.clone();
            }
            None if !retry.saving() => {
                let username = retry.credential.username.clone();
                if let Some(screen) = self.screen_mut(session) {
                    *screen = Screen::SignIn(SignIn::new(username, Some(DELETED.into())));
                }
            }
            None => {}
        }
    }

    /// Next ID for a store request.
    fn request(&mut self) -> RequestId {
        self.next_request += 1;
        self.next_request
    }

    fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
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

    /// A New host in a new Tab or Session window, with the sign-in sheet.
    pub fn open_host(&mut self, address: Address, opening: Opening) -> SessionId {
        let id = self.add(Destination::new_host(address), opening);
        self.ask(id, None);
        id
    }

    /// Switches to the Connection's Session if one is open, or opens one, signing in silently
    /// when it has a Credential. With `another`, opens one regardless.
    pub fn open(&mut self, connection: &ConnectionId, opening: Opening, another: bool) {
        let Some(snapshot) = self.library.ready() else { return };
        let Some(found) = snapshot.connection(connection) else {
            return;
        };
        let to = Destination::connection(found);
        let mine = |ended: bool| {
            self.sessions
                .iter()
                .filter(|s| s.to.connection.as_ref() == Some(connection))
                .find(|s| matches!(s.screen, Screen::Ended(_)) == ended)
                .map(|s| (s.id, s.place.is_tab()))
        };
        // A Session still open wins over one that ended.
        let (open, ended) = match another {
            true => (None, None),
            false => (mine(false), mine(true)),
        };
        let id = match (open, ended) {
            (Some((id, true)), _) if opening == Opening::Window => return self.move_to_window(id, None),
            (Some((id, _)), _) => return self.reveal(id),
            // A Session that ended is opened again, where it was asked for.
            (None, Some((id, tab))) => {
                match (opening, tab) {
                    (Opening::Window, true) => self.move_to_window(id, None),
                    (Opening::Tab, false) => self.move_to_main(id),
                    (Opening::Tab, true) | (Opening::Window, false) => {}
                }
                if let Some(session) = self.session_mut(id) {
                    session.to = to;
                }
                id
            }
            (None, None) => self.add(to, opening),
        };
        self.start(id);
        self.reveal(id);
    }

    /// A new Session for `to`, in a new Tab that becomes the current one or a new Session
    /// window. It shows the sign-in sheet until something else starts it.
    fn add(&mut self, to: Destination, opening: Opening) -> SessionId {
        let id = self.next_id();
        let screen = Screen::SignIn(SignIn::new(self.last_username.clone(), None));
        self.sessions.push(Session {
            id,
            to,
            place: Place::Tab,
            screen,
            notice: None,
            assigning: None,
        });
        match opening {
            Opening::Tab => self.activate(id),
            Opening::Window => self.open_window(id, None),
        }
        id
    }

    /// Signs the Session in to its Connection: silently with its Credential, else through the
    /// sign-in sheet.
    fn start(&mut self, id: SessionId) {
        let credential = self.session(id).and_then(|session| {
            let snapshot = self.library.ready()?;
            let connection = snapshot.connection(session.to.connection.as_ref()?)?;
            Some(snapshot.credential_of(connection)?.id.clone())
        });
        match credential {
            Some(credential) => self.unlock(id, credential),
            None => self.ask(id, None),
        }
    }

    /// Brings the Session to the front: its Tab in the Main window, or its Session window.
    pub fn reveal(&mut self, id: SessionId) {
        let Some(session) = self.session(id) else { return };
        match &session.place {
            Place::Tab => {
                self.activate(id);
                self.show_main();
            }
            Place::Window(window) => window.focus(),
        }
    }

    /// Makes the Tab the Main window's current one, sized to the Main window's area.
    pub fn activate(&mut self, id: SessionId) {
        // The outgoing Tab's desktop leaves the tree, so its key releases never arrive.
        if let Some(old) = self.active.filter(|old| *old != id) {
            self.release_keys(old);
        }
        self.active = Some(id);
        self.resize(id, self.area_of(Window::Main));
    }

    pub(crate) fn area_of(&self, window: Window) -> Option<View> {
        self.areas.iter().find(|(w, _)| *w == window).map(|(_, v)| *v)
    }

    pub(crate) fn resize(&self, id: SessionId, area: Option<View>) {
        if let (Some(live), Some(area)) = (self.session(id).and_then(Session::live), area) {
            live.handle.resize(area);
        }
    }

    /// Shows the Main window when another window asks for it.
    pub(crate) fn show_main(&self) {
        if self.here.window != Window::Main
            && let Some(main) = &self.main
        {
            main.open();
        }
    }

    /// The status dot of a Connection with a Session on its way or open: green when any of
    /// its Sessions is connected.
    pub fn dot(&self, id: &ConnectionId) -> Option<Dot> {
        self.sessions
            .iter()
            .filter(|s| s.to.connection.as_ref() == Some(id))
            .filter_map(Session::dot)
            .max_by_key(|dot| *dot == Dot::Connected)
    }

    /// Ends the Session and closes its Tab or Session window; Windows keeps the user signed in.
    pub fn close(&mut self, id: SessionId) {
        if let Some(Place::Window(window)) = self.end(id) {
            window.close();
        }
    }

    /// Ends the Session and takes it off the tab row; returns where it was.
    pub(crate) fn end(&mut self, id: SessionId) -> Option<Place> {
        let at = self.sessions.iter().position(|s| s.id == id)?;
        let session = self.sessions.remove(at);
        if let Screen::Live(live) = &session.screen {
            live.handle.disconnect();
        }
        if self.active == Some(id) {
            self.after_tab(at);
        }
        Some(session.place)
    }

    fn unlock(&mut self, id: SessionId, credential: CredentialId) {
        let request = self.request();
        if let Some(store) = &self.store {
            store.password(request, credential.clone());
        }
        if let Some(screen) = self.screen_mut(id) {
            *screen = Screen::Unlocking(Unlocking { credential, request });
        }
    }

    /// Connects with a saved Credential's password, as the account it was read or typed for.
    fn connect_as(&mut self, id: SessionId, credential: Credential, password: Secret, assign: bool) {
        match Username::parse(&credential.username) {
            Ok(username) => self.connect(
                id,
                Connect {
                    username,
                    password,
                    signing: Some(Signing {
                        credential,
                        adding: None,
                        assign,
                    }),
                },
            ),
            Err(e) => self.ask(id, Some(format!("{} has an unusable username: {e}", credential.name))),
        }
    }

    /// Once a Session that saved its Credential from the sign-in sheet is signed in and the
    /// Credential is saved, makes the Connection it signed in to use it from now on, if that
    /// Connection still asks each time and reaches the same endpoint.
    fn assign_when_signed_in(&mut self, id: SessionId) {
        let Some(session) = self.session_mut(id) else { return };
        let Screen::Live(live) = &mut session.screen else {
            return;
        };
        let (Some(signing), Some(connection)) = (&mut live.signing, &session.to.connection) else {
            return;
        };
        if !signing.assign || signing.adding.is_some() || !matches!(live.state, Phase::Connected { .. }) {
            return;
        }
        signing.assign = false;
        let change = Change::AssignCredential {
            connection: connection.clone(),
            credential: signing.credential.id.clone(),
            host: session.to.address.host.clone(),
            port: session.to.address.port,
        };
        let name = signing.credential.name.clone();
        let request = self.apply(change);
        if let Some(session) = self.session_mut(id) {
            session.assigning = request.map(|request| (request, name));
        }
    }

    /// Cancel on the sign-in sheet or the wrong-password prompt: the Session never connected,
    /// so its Tab or Session window closes.
    pub fn cancel_sign_in(&mut self, id: SessionId) {
        self.close(id);
    }

    /// Picks a saved Credential in the sign-in sheet, or `None` to type one.
    pub fn sign_in_with(&mut self, id: SessionId, credential: Option<CredentialId>) {
        if let Some(Screen::SignIn(sign_in)) = self.screen_mut(id) {
            sign_in.credential = credential;
            sign_in.error = None;
        }
    }

    pub fn toggle_save_credential(&mut self, id: SessionId) {
        if let Some(Screen::SignIn(sign_in)) = self.screen_mut(id) {
            sign_in.save = !sign_in.save;
            sign_in.save_name.focused = sign_in.save;
        }
    }

    pub fn sign_in(&mut self, id: SessionId) {
        let Some(Session {
            to,
            screen: Screen::SignIn(sign_in),
            ..
        }) = self.sessions.iter_mut().find(|s| s.id == id)
        else {
            return;
        };
        if sign_in.waiting.is_some() {
            return;
        }
        if let Some(credential) = sign_in.credential.clone() {
            return self.unlock(id, credential);
        }
        let username = match Username::parse(&sign_in.username.text) {
            Ok(username) => username,
            Err(e) => return sign_in.error = Some(e.to_string()),
        };
        let save_name = sign_in.save_name.text.trim().to_owned();
        if sign_in.save && save_name.is_empty() {
            return sign_in.error = Some("Enter a name for the credential".into());
        }
        let assign = to.connection.is_some();
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
                assign,
            })
        } else {
            None
        };
        let connect = Connect {
            username,
            password,
            signing,
        };
        match self.library {
            Library::Loading => {
                if let Some(Screen::SignIn(sign_in)) = self.screen_mut(id) {
                    sign_in.waiting = Some(connect);
                }
            }
            Library::Ready(_) | Library::Failed(_) => self.connect(id, connect),
        }
    }

    fn connect(&mut self, id: SessionId, connect: Connect) {
        let Connect {
            username,
            password,
            signing,
        } = connect;
        let Some(session) = self.session(id) else { return };
        let to = &session.to;
        let window = match &session.place {
            Place::Tab => Window::Main,
            Place::Window(_) => Window::Session(id),
        };
        let (trusted_certificate, command_key) = match &self.library {
            Library::Ready(snapshot) => (
                snapshot.trusted_certificate(&to.address.host, to.address.port).cloned(),
                snapshot.settings.command_key,
            ),
            Library::Loading | Library::Failed(_) => (None, Default::default()),
        };
        let target = Target {
            host: to.address.host.clone(),
            port: to.address.port,
            view: self.area_of(window).unwrap_or(FALLBACK_VIEW),
            trusted_certificate,
        };
        let engine = self.next_id();
        let (outbox, alarm) = (self.outbox.clone(), self.alarm.clone());
        let display_name = username.to_string();
        let handle = bezel_rdp::Session::connect(
            target,
            Auth {
                username,
                password: password.into_string().into(),
            },
            move |event| {
                let _ = outbox.send(Message::Engine(engine, event));
                alarm.ring();
            },
        );
        if let Some(session) = self.session_mut(id) {
            session.screen = Screen::Live(Live {
                engine,
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
    }

    /// An event from the engine `engine`, for the Session it belongs to.
    fn on_engine(&mut self, engine: u64, event: Event) {
        let found = self
            .sessions
            .iter()
            .find(|s| matches!(&s.screen, Screen::Live(live) if live.engine == engine))
            .map(|s| s.id);
        if let Some(id) = found {
            self.on_session(id, event);
        }
    }

    fn on_session(&mut self, id: SessionId, event: Event) {
        let remember = self.library.ready().is_some_and(|s| s.settings.remember_certificates);
        let Some(session) = self.session_mut(id) else { return };
        let Screen::Live(live) = &mut session.screen else {
            return;
        };
        match event {
            Event::State(State::Connecting) => live.state = Phase::Connecting,
            Event::State(State::Connected { width, height }) => {
                live.state = Phase::Connected { width, height };
                self.assign_when_signed_in(id);
            }
            Event::State(State::Reconnecting) => live.state = Phase::Reconnecting,
            Event::State(State::Disconnected(ended)) => {
                if ended.by_user {
                    self.close(id);
                } else {
                    session.screen = Screen::Ended(ended);
                }
            }
            Event::Pointer(pointer) => live.pointer = pointer,
            Event::Certificate(request) => {
                live.certificate = Some(Prompt {
                    request,
                    remember,
                    confirming: false,
                })
            }
            Event::SignInFailed { wrong_password, ended } => {
                session.screen = match live.signing.clone() {
                    Some(signing) if wrong_password => {
                        let mut retry = retry(signing.credential, signing.assign, true, None);
                        retry.adding = signing.adding;
                        Screen::Retry(retry)
                    }
                    Some(_) => Screen::Ended(ended),
                    // The account this Session typed, not whichever was typed last anywhere.
                    None => Screen::SignIn(SignIn::new(live.username.clone(), Some(ended.message))),
                };
                // The Credential may have changed or gone while this Session signed in.
                self.follow_retry(id);
            }
        }
    }

    /// "Save and retry" on the wrong-password prompt: the password is saved to the Credential,
    /// for every Connection that uses it, and the Connection signs in again once it is.
    pub fn retry(&mut self, id: SessionId) {
        let Some(Screen::Retry(retry)) = self.screen_mut(id) else {
            return;
        };
        if retry.saving() || retry.password.text.is_empty() {
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
        if let Some(request) = self.apply(change)
            && let Some(Screen::Retry(retry)) = self.screen_mut(id)
        {
            retry.saving = Some((request, password, retry.credential.clone()));
        }
    }

    fn live_mut(&mut self, id: SessionId) -> Option<&mut Live> {
        match self.screen_mut(id)? {
            Screen::Live(live) => Some(live),
            _ => None,
        }
    }

    pub fn toggle_remember(&mut self, id: SessionId) {
        if let Some(prompt) = self.live_mut(id).and_then(|l| l.certificate.as_mut()) {
            prompt.remember = !prompt.remember;
        }
    }

    pub fn confirm_change(&mut self, id: SessionId, confirming: bool) {
        if let Some(prompt) = self.live_mut(id).and_then(|l| l.certificate.as_mut()) {
            prompt.confirming = confirming;
        }
    }

    /// "Trust and connect" for a first use, or the confirmed "Trust new certificate" for a change.
    pub fn trust(&mut self, id: SessionId) {
        let Some(prompt) = self.live_mut(id).and_then(|l| l.certificate.take()) else {
            return;
        };
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
        let mut notice = None;
        let trust = match change {
            None => Trust::ThisSession,
            Some(change) => match self.apply(change) {
                Some(request) => Trust::Remembering(request),
                None => {
                    let why = match &self.library {
                        Library::Failed(e) => format!(" {e}"),
                        Library::Loading | Library::Ready(_) => String::new(),
                    };
                    notice = Some(format!(
                        "The certificate is trusted for this session but was not remembered.{why}"
                    ));
                    Trust::ThisSession
                }
            },
        };
        if let Some(session) = self.session_mut(id) {
            if let Screen::Live(live) = &mut session.screen {
                live.trust = trust;
            }
            if notice.is_some() {
                session.notice = notice;
            }
        }
    }

    pub fn refuse_certificate(&mut self, id: SessionId) {
        if let Some(prompt) = self.live_mut(id).and_then(|l| l.certificate.take()) {
            prompt.request.cancel();
        }
    }

    pub fn dismiss_notice(&mut self) {
        self.notice = None;
    }

    pub fn dismiss_session_notice(&mut self, id: SessionId) {
        if let Some(session) = self.session_mut(id) {
            session.notice = None;
        }
    }

    pub fn send_ctrl_alt_del(&mut self, id: SessionId) {
        if let Some(live) = self.session(id).and_then(Session::live) {
            live.handle.send_ctrl_alt_del();
        }
    }

    /// ⌃⌘F. The tab row starts hidden in full screen until the pointer reaches the top edge.
    pub fn toggle_full_screen(&mut self, tx: &mut Tx) {
        self.here.reveal = false;
        tx.toggle_full_screen();
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar = !self.sidebar;
    }

    /// The window's content area was laid out: the Session it shows fills it.
    pub fn layout_area(&mut self, rect: [f32; 4], tx: &mut Tx) {
        let view = View {
            width: rect[2],
            height: rect[3],
            scale: tx.scale(),
        };
        let window = self.here.window;
        if self.area_of(window) == Some(view) {
            return;
        }
        match self.areas.iter_mut().find(|(w, _)| *w == window) {
            Some((_, area)) => *area = view,
            None => self.areas.push((window, view)),
        }
        if let Some(id) = self.current() {
            self.resize(id, Some(view));
        }
    }

    pub fn input(&self, id: SessionId, input: Input) {
        if let Some(live) = self.session(id).and_then(Session::live) {
            live.handle.input(input);
        }
    }

    /// Keys pressed over a Session's desktop. True when Bezel or the Session took the key.
    pub fn physical(&mut self, id: SessionId, event: &PhysicalInput, tx: &mut Tx) -> bool {
        let Some(live) = self.live_mut(id) else {
            return false;
        };
        match keymap::translate(&mut live.keyboard, event) {
            Translated::Key(held, outcome) => {
                if !held.is_empty() {
                    live.handle.input(Input::Keys(held));
                }
                match outcome {
                    KeyOutcome::Send(ops) => {
                        if !ops.is_empty() {
                            live.handle.input(Input::Keys(ops));
                        }
                        true
                    }
                    KeyOutcome::Shortcut(shortcut) => self.shortcut(shortcut, tx),
                }
            }
            Translated::LetGo(ops) => {
                live.handle.input(Input::Keys(ops));
                false
            }
            Translated::Pass => false,
            Translated::FocusLost => {
                live.handle.input(Input::ReleaseAll);
                false
            }
        }
    }

    /// A Bezel shortcut in this window. True when Bezel took it; ⌘Q and ⌘, are left to Rune
    /// and the menus.
    fn shortcut(&mut self, shortcut: Shortcut, tx: &mut Tx) -> bool {
        match shortcut {
            Shortcut::Palette => self.open_palette(tx),
            Shortcut::Close => match self.current() {
                Some(id) => self.close(id),
                None => return false,
            },
            Shortcut::Sidebar => self.toggle_sidebar(),
            Shortcut::FullScreen => self.toggle_full_screen(tx),
            Shortcut::PreviousTab => self.step_tab(-1),
            Shortcut::NextTab => self.step_tab(1),
            Shortcut::Tab(n) => self.nth_tab(usize::from(n)),
            Shortcut::Quit | Shortcut::Settings => return false,
        }
        true
    }

    /// Moves to the Tab `step` places from the current one, wrapping around.
    fn step_tab(&mut self, step: isize) {
        let tabs: Vec<SessionId> = self.tabs().map(|s| s.id).collect();
        let Some(at) = self.active.and_then(|id| tabs.iter().position(|t| *t == id)) else {
            return;
        };
        let next = (at as isize + step).rem_euclid(tabs.len() as isize) as usize;
        self.reveal(tabs[next]);
    }

    /// ⌘1 to ⌘8 pick that Tab; ⌘9 the last.
    fn nth_tab(&mut self, n: usize) {
        let tabs: Vec<SessionId> = self.tabs().map(|s| s.id).collect();
        let pick = if n >= 9 {
            tabs.last()
        } else {
            tabs.get(n.saturating_sub(1))
        };
        if let Some(id) = pick.copied() {
            self.reveal(id);
        }
    }

    /// Keys anywhere in the window that no control took.
    pub fn on_key(&mut self, event: &KeyEvent, tx: &mut Tx) -> bool {
        let mods = event.mods;
        // With ⌃ held the key may arrive as its control character (⌃S is U+0013, ⌃F U+0006).
        let shortcut = match event.key {
            Key::Command('s' | '\u{13}') if mods.ctrl && !mods.shift && !mods.alt => Shortcut::Sidebar,
            Key::Command('f' | '\u{6}') if mods.ctrl && !mods.shift && !mods.alt => Shortcut::FullScreen,
            Key::Command('k' | 't') if !mods.ctrl && !mods.shift && !mods.alt => Shortcut::Palette,
            Key::Command('w') if !mods.ctrl && !mods.shift && !mods.alt => Shortcut::Close,
            Key::Command('[' | '{') if mods.shift && !mods.ctrl => Shortcut::PreviousTab,
            Key::Command(']' | '}') if mods.shift && !mods.ctrl => Shortcut::NextTab,
            Key::Command(c @ '1'..='9') if !mods.ctrl && !mods.shift && !mods.alt => Shortcut::Tab(c as u8 - b'0'),
            _ => return false,
        };
        self.shortcut(shortcut, tx)
    }
}

/// Shown on the sign-in sheet when the Credential a sign-in needs is gone.
const DELETED: &str = "The credential was deleted. Sign in another way.";

fn retry(credential: Credential, assign: bool, rejected: bool, error: Option<String>) -> Retry {
    let mut password = TextEdit::default().secure();
    password.focused = true;
    Retry {
        credential,
        assign,
        adding: None,
        rejected,
        password,
        error,
        saving: None,
    }
}

/// For fixture scenes: a Session with nothing behind it.
pub fn fixture_session(
    id: SessionId,
    to: Destination,
    state: Phase,
    certificate: Option<CertificateRequest>,
) -> Session {
    Session {
        id,
        to,
        place: Place::Tab,
        screen: Screen::Live(Live {
            engine: 0,
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
        }),
        notice: None,
        assigning: None,
    }
}

/// For fixture scenes: the wrong-password prompt.
pub fn fixture_retry(credential: Credential) -> Screen {
    Screen::Retry(retry(credential, false, true, None))
}

/// For fixture scenes: the sign-in sheet.
pub fn fixture_sign_in() -> Screen {
    Screen::SignIn(SignIn::new(String::new(), None))
}

pub fn today() -> time::Date {
    time::OffsetDateTime::now_utc().date()
}

#[cfg(test)]
mod tests;
