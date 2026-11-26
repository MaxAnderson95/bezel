//! Bezel's windows: the Main window and a Session window per Session moved out of its Tab.
//!
//! Rune gives each window a state of its own, while every Bezel window shows the same Library
//! and Sessions. So each window's state is a stand-in (`Bezel::husk`) holding only what is the
//! window's own (`Here`), and the shared state waits in `Home` between turns: at the start of a
//! window's turn (`App::turn_scope`) the shared state moves into the window's state, keeping the
//! window's `Here`, and moves back out when the turn ends. Rune runs one window's turn at a time.
//! A Session moves between a Tab and a Session window by changing its `Place` in the shared
//! state, so its live connection never notices. Headless runs have one window and no turns, so
//! there the window's state is the shared state itself.

use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use bezel_rdp::Input;
use rune::{AppSpec, Chrome, CloseResponse, Drag, Phase, QuitResponse, Tx, Waker, WindowHandle, WindowSpec};

use crate::app::{Bezel, Place, Screen, SessionId};
use crate::palette::Palette;

pub type Home = Rc<RefCell<Bezel>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Window {
    Main,
    Session(SessionId),
}

/// What belongs to one window, kept in its own state through every turn.
pub struct Here {
    pub window: Window,
    /// The native window's key: `main`, or one per Session window opened.
    pub key: String,
    pub(crate) waker: Option<Waker>,
    /// The shared state's `generation` this window last drew.
    pub(crate) seen: u64,
    pub palette: Palette,
    /// Full screen: the pointer reached the top edge, so the tab row shows.
    pub reveal: bool,
    /// A Session window's title bar held by the pointer.
    pub(crate) grab: Option<Grab>,
    title: String,
}

/// Where a dragged Session window and the pointer were when the drag began, in desktop pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Grab {
    window: (i32, i32),
    pointer: (i32, i32),
}

impl Here {
    pub fn main() -> Self {
        Self::new(Window::Main, "main".into())
    }

    fn new(window: Window, key: String) -> Self {
        Self {
            window,
            key,
            waker: None,
            seen: 0,
            palette: Palette::default(),
            reveal: false,
            grab: None,
            title: String::new(),
        }
    }
}

/// Wakes every window, from any thread: something arrived that any of them may show.
#[derive(Clone, Default)]
pub struct Alarm(Arc<Mutex<Vec<(String, Waker)>>>);

impl Alarm {
    pub fn ring(&self) {
        let wakers: Vec<Waker> = self
            .0
            .lock()
            .map(|w| w.iter().map(|(_, w)| w.clone()).collect())
            .unwrap_or_default();
        for wake in wakers {
            wake();
        }
    }

    pub fn set(&self, key: &str, waker: Waker) {
        if let Ok(mut wakers) = self.0.lock() {
            wakers.retain(|(k, _)| k != key);
            wakers.push((key.to_owned(), waker));
        }
    }

    fn remove(&self, key: &str) {
        if let Ok(mut wakers) = self.0.lock() {
            wakers.retain(|(k, _)| k != key);
        }
    }
}

/// `App::turn_scope`'s start: the shared state moves into this window's state.
pub fn enter(s: &mut Bezel) {
    let Some(home) = s.home.clone() else { return };
    let mut shared = home.borrow_mut();
    std::mem::swap(s, &mut shared);
    std::mem::swap(&mut s.here, &mut shared.here);
}

/// `App::turn_scope`'s end: the shared state goes home, and the other windows hear of any
/// change they show.
pub fn leave(s: &mut Bezel) {
    let Some(home) = s.home.clone() else { return };
    let outline = outline(s);
    if outline != s.outline {
        s.outline = outline;
        s.changed();
    }
    let mut stand_in = home.borrow_mut();
    std::mem::swap(&mut s.here, &mut stand_in.here);
    std::mem::swap(s, &mut stand_in);
}

/// A digest of what a window shows of state another window can change: the Sessions, their
/// places, screens and dots, the Main window's Tab, sheets and confirmations. Store replies
/// and engine events reach every window through `poll` instead.
fn outline(s: &Bezel) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for session in &s.sessions {
        session.id.hash(&mut hash);
        session.to.name.hash(&mut hash);
        session.place.is_tab().hash(&mut hash);
        std::mem::discriminant(&session.screen).hash(&mut hash);
        session.dot().hash(&mut hash);
        session.notice.is_some().hash(&mut hash);
    }
    s.active.hash(&mut hash);
    s.sheet.as_ref().map(std::mem::discriminant).hash(&mut hash);
    s.deleting.is_some().hash(&mut hash);
    s.notice.hash(&mut hash);
    s.quitting.hash(&mut hash);
    hash.finish()
}

/// The Session window's thin title bar, in points.
pub const TITLE_BAR: f32 = 32.0;

impl Bezel {
    /// A window's stand-in for the shared state at `home`.
    pub fn husk(window: Window, key: String, home: Home) -> Self {
        let mut husk = Bezel::new();
        husk.alarm = home.borrow().alarm.clone();
        husk.here = Here::new(window, key);
        husk.home = Some(home);
        husk
    }

    /// Something another window shows changed: every window draws again.
    pub(crate) fn changed(&mut self) {
        self.generation += 1;
        self.outline = outline(self);
        self.alarm.ring();
    }

    /// Names a Session window after its Session.
    pub(crate) fn title_window(&mut self, tx: &mut Tx) {
        let Window::Session(id) = self.here.window else { return };
        let Some(name) = self.session(id).map(|s| s.to.name.clone()) else {
            return;
        };
        if self.here.title != name {
            tx.set_title(name.clone());
            self.here.title = name;
        }
    }

    /// Opens a Session window for the Session, at `at` (the frame's top-left in desktop pixels)
    /// or where the system puts it.
    pub(crate) fn open_window(&mut self, id: SessionId, at: Option<(i32, i32)>) {
        // A size from an earlier window of this Session would let the new one skip resizing it.
        self.areas.retain(|(w, _)| *w != Window::Session(id));
        self.next_window += 1;
        let key = format!("session-{}", self.next_window);
        let size = self
            .area_of(Window::Main)
            .map_or((1024.0, 680.0), |v| (v.width.max(640.0), v.height + TITLE_BAR));
        let spec = AppSpec {
            min_size: Some((480.0, 320.0)),
            fonts: rune_theme_opencode::FONTS,
            families: rune_theme_opencode::FAMILIES,
            setup: Some(Bezel::setup),
            poll: Some(Bezel::poll),
            on_key: Some(Bezel::on_key),
            ..AppSpec::new("Bezel", size, Bezel::new, crate::view::view)
        };
        let mut window = WindowSpec::new(key.clone(), spec)
            .chrome(Chrome {
                full_size_content: true,
                traffic_lights: Some((12.0, (TITLE_BAR - Chrome::TRAFFIC_LIGHT) / 2.0)),
                drag_regions: true,
            })
            .turn_scope(enter, leave)
            .on_close(on_window_closed)
            .on_quit_request(on_quit_request);
        window.options.position = at;
        if let Some(home) = &self.home {
            window = window.state(Bezel::husk(Window::Session(id), key, home.clone()));
        }
        let handle = rune::open_window(window);
        if let Some(session) = self.session_mut(id) {
            session.place = Place::Window(handle);
        }
    }

    /// "Move to new window", or a Tab dragged off the tab row: the Session, still connected,
    /// moves to a Session window of its own.
    pub fn move_to_window(&mut self, id: SessionId, at: Option<(i32, i32)>) {
        let Some(index) = self.sessions.iter().position(|s| s.id == id && s.place.is_tab()) else {
            return;
        };
        self.release_keys(id);
        self.open_window(id, at);
        if self.active == Some(id) {
            self.after_tab(index);
        }
    }

    /// "Move to main window", or a Session window dropped on the tab row: the Session, still
    /// connected, becomes the Main window's last Tab and its window closes.
    pub fn move_to_main(&mut self, id: SessionId) {
        let Some(index) = self.sessions.iter().position(|s| s.id == id && !s.place.is_tab()) else {
            return;
        };
        self.release_keys(id);
        let mut session = self.sessions.remove(index);
        let window = std::mem::replace(&mut session.place, Place::Tab);
        self.sessions.push(session);
        self.activate(id);
        if let Place::Window(window) = window {
            window.close();
        }
        if let Some(main) = &self.main {
            main.open();
        }
    }

    /// Lets go of every key the Session holds: the desktop it leaves won't send their releases.
    pub(crate) fn release_keys(&mut self, id: SessionId) {
        if let Some(Screen::Live(live)) = self.session_mut(id).map(|s| &mut s.screen) {
            live.release_keys();
            live.handle.input(Input::ReleaseAll);
        }
    }

    /// The current Tab at `index` left the tab row: its neighbour becomes current.
    pub(crate) fn after_tab(&mut self, index: usize) {
        self.active = None;
        let tabs: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|s| s.place.is_tab())
            .map(|s| s.id)
            .collect();
        let before = self.sessions[..index.min(self.sessions.len())]
            .iter()
            .filter(|s| s.place.is_tab())
            .count();
        let next = tabs.get(before.saturating_sub(1)).or(tabs.first()).copied();
        if let Some(next) = next {
            self.activate(next);
        }
    }

    /// Moves the Tab at place `from` among the Tabs to place `to`.
    pub fn reorder_tab(&mut self, from: usize, to: usize) {
        let tabs: Vec<usize> = (0..self.sessions.len())
            .filter(|i| self.sessions[*i].place.is_tab())
            .collect();
        let (Some(&from), Some(&to)) = (tabs.get(from), tabs.get(to)) else {
            return;
        };
        let session = self.sessions.remove(from);
        self.sessions.insert(to, session);
    }

    /// A Session window's title bar dragged: the window follows the pointer, and let go over
    /// the Main window's tab row it becomes a Tab again.
    pub fn drag_window(&mut self, id: SessionId, d: &Drag, tx: &mut Tx) {
        let Some(Place::Window(window)) = self.session(id).map(|s| &s.place) else {
            return;
        };
        match d.phase {
            Phase::Down => {
                self.here.grab = window
                    .placement()
                    .zip(d.screen_point())
                    .map(|(placement, pointer)| Grab {
                        window: placement.position,
                        pointer,
                    });
            }
            Phase::Move => {
                if let (Some(grab), Some(at)) = (self.here.grab, d.screen_point()) {
                    window.set_position((
                        grab.window.0 + at.0 - grab.pointer.0,
                        grab.window.1 + at.1 - grab.pointer.1,
                    ));
                }
                tx.live_only();
            }
            Phase::Up => {
                let dropped = self.here.grab.take().is_some() && !d.cancelled();
                if dropped
                    && let Some(at) = d.screen_point()
                    && self.over_tab_row(at)
                {
                    self.move_to_main(id);
                }
            }
        }
    }

    fn over_tab_row(&self, at: (i32, i32)) -> bool {
        let main = self.main_window.borrow();
        let (Some(main), Some(row)) = (main.as_ref(), self.tab_row) else {
            return false;
        };
        main.placement()
            .is_some_and(|p| !p.minimized && lands_on(at, p.content_position, p.scale, row))
    }

    /// The red button or ⌘Q on the Main window: it hides, and its Sessions keep running.
    pub fn hide_main(&mut self) {
        if let Some(main) = &self.main {
            main.hide();
        }
    }

    /// "Disconnect and quit" on the ⌘Q question.
    pub fn quit(&mut self, tx: &mut Tx) {
        for id in self.sessions.iter().map(|s| s.id).collect::<Vec<_>>() {
            self.end(id);
        }
        self.quitting = false;
        tx.quit();
    }

    pub fn cancel_quit(&mut self) {
        self.quitting = false;
    }
}

/// Whether a point on the desktop falls on `rect` (window points) of a window whose content
/// starts at `origin` (desktop pixels) with `scale` pixels per point.
fn lands_on(at: (i32, i32), origin: (i32, i32), scale: f64, rect: [f32; 4]) -> bool {
    let x = (f64::from(at.0 - origin.0)) / scale;
    let y = (f64::from(at.1 - origin.1)) / scale;
    let [rx, ry, w, h] = rect.map(f64::from);
    x >= rx && x < rx + w && y >= ry && y < ry + h
}

/// The Main window's close button hides it; its Sessions keep running.
pub fn on_main_close_request(s: &mut Bezel, _: &mut Tx) -> CloseResponse {
    s.hide_main();
    CloseResponse::Defer
}

/// A Session window closed by its close button or ⌘W ends its Session; one closing because
/// its Session moved to a Tab leaves the Session alone.
fn on_window_closed(s: &mut Bezel, _: &mut Tx) {
    s.alarm.remove(&s.here.key);
    let Window::Session(id) = s.here.window else { return };
    s.areas.retain(|(w, _)| *w != Window::Session(id));
    let mine = matches!(s.session(id).map(|x| &x.place), Some(Place::Window(w)) if w.key() == s.here.key);
    if mine {
        s.end(id);
    }
}

/// Clicking the Dock icon shows the Main window, also while Session windows are on screen.
pub fn on_reopen(s: &mut Bezel, _: bool, _: &mut Tx) -> bool {
    match &s.main {
        Some(main) => {
            main.open();
            true
        }
        None => false,
    }
}

/// ⌘Q asks once, in the Main window, when Sessions are open.
pub fn on_quit_request(s: &mut Bezel, _: &mut Tx) -> QuitResponse {
    // The app menu takes ⌘Q before the desktop sees Q, so the desktop holding ⌘ would make its
    // key-up a lone Windows key tap.
    for id in s.sessions.iter().map(|x| x.id).collect::<Vec<_>>() {
        s.release_keys(id);
    }
    if !s.sessions.iter().any(|x| x.is_open()) {
        return QuitResponse::Quit;
    }
    s.quitting = true;
    // The question takes the keyboard, not a palette left open in this window.
    s.close_palette();
    if let Some(main) = &s.main {
        main.open();
    }
    QuitResponse::Defer
}

/// The shared state for a native run, its Main window's stand-in, and the cell the Main
/// window's handle lands in once the window exists.
pub fn home() -> (Home, Bezel, Rc<RefCell<Option<WindowHandle>>>) {
    let mut shared = Bezel::new();
    shared.open_store();
    let main_window = shared.main_window.clone();
    let home = Rc::new(RefCell::new(shared));
    // The shared state knows its home too, as it is the one in a window's state during turns.
    home.borrow_mut().home = Some(home.clone());
    let husk = Bezel::husk(Window::Main, "main".into(), home.clone());
    (home, husk, main_window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_dropped_on_the_tab_row_lands_by_its_desktop_position() {
        let row = [80.0, 0.0, 600.0, 40.0];
        // The Main window's content starts at (1000, 500) on a 2x display.
        assert!(lands_on((1000 + 200, 500 + 40), (1000, 500), 2.0, row));
        assert!(!lands_on((1000 + 200, 500 + 90), (1000, 500), 2.0, row));
        assert!(!lands_on((1000 + 100, 500 + 10), (1000, 500), 2.0, row));
    }

    #[test]
    fn turns_move_the_shared_state_into_each_window_and_back() {
        let mut shared = Bezel::new();
        shared.next_window = 42;
        let home = Rc::new(RefCell::new(shared));
        home.borrow_mut().home = Some(home.clone());
        let mut main = Bezel::husk(Window::Main, "main".into(), home.clone());
        let mut other = Bezel::husk(Window::Session(7), "session-1".into(), home.clone());
        main.here.reveal = true;

        enter(&mut main);
        assert_eq!(main.next_window, 42, "the shared state is in the Main window's turn");
        assert_eq!(main.here.window, Window::Main);
        assert!(main.here.reveal, "the window keeps its own");
        main.next_window = 43;
        leave(&mut main);
        assert_eq!(main.next_window, 0, "between turns the window holds its stand-in");

        enter(&mut other);
        assert_eq!(
            other.next_window, 43,
            "a change in one turn is there in the next window's"
        );
        assert_eq!(other.here.window, Window::Session(7));
        assert!(!other.here.reveal);
        leave(&mut other);

        assert_eq!(
            home.borrow().next_window,
            43,
            "home holds the shared state between turns"
        );
        assert_eq!(
            (main.here.window, other.here.window),
            (Window::Main, Window::Session(7))
        );
        assert!(main.here.reveal);
    }
}
