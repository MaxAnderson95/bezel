//! The command palette (⌘K): what typing matches, in the order Open, Saved, New host and
//! Manage, and what Return, ⌘Return and ⌥Return do with the selection. With no Tab open the
//! Main window's panel shows the same search field inline.

use bezel_core::address::Address;
use bezel_core::library::{ConnectionId, Snapshot};
use rune::{El, Key, KeyEvent, TextEdit, Tx, col, el, row, text};
use rune_kit::{Keybind, Kit, TextInput};
use rune_theme_opencode::icons;

use crate::app::{Bezel, Opening, Screen, Session, SessionId};
use crate::manage::{Sheet, host_text};
use crate::windows::Window;

#[derive(Default)]
pub struct Palette {
    /// Floating over the window; the inline field in an empty Main window needs no opening.
    pub open: bool,
    pub query: TextEdit,
    pub selected: usize,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Open,
    Saved,
    NewHost,
    Manage,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Section::Open => "Open",
            Section::Saved => "Saved",
            Section::NewHost => "New host",
            Section::Manage => "Manage",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Open(SessionId),
    Saved(ConnectionId),
    NewHost(String),
    Edit(ConnectionId),
    Credentials,
    Certificates,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub section: Section,
    pub item: Item,
    pub title: String,
    pub detail: String,
}

/// How the selection was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    /// Return: open in a new Tab, or switch to it.
    Return,
    /// ⌘Return: open in a Session window.
    Command,
    /// ⌥Return: edit a Saved Connection or New host, or open another Session of an open one.
    Option,
}

impl How {
    fn of(event: &KeyEvent) -> Self {
        if event.mods.cmd {
            How::Command
        } else if event.mods.alt {
            How::Option
        } else {
            How::Return
        }
    }
}

/// What `query` matches, section by section. An empty query lists the open Sessions and every
/// Connection; Edit items and a New host need something typed.
pub fn rows(query: &str, sessions: &[Session], library: Option<&Snapshot>) -> Vec<Row> {
    let typed = query.trim();
    let needle = typed.to_lowercase();
    let hit = |texts: &[&str]| needle.is_empty() || texts.iter().any(|t| t.to_lowercase().contains(&needle));
    let mut rows = Vec::new();
    // A Session that ended isn't open; its Connection's Saved row starts it again.
    for session in sessions.iter().filter(|s| !matches!(s.screen, Screen::Ended(_))) {
        let host = &session.to.address.host;
        if hit(&[&session.to.name, host]) {
            rows.push(Row {
                section: Section::Open,
                item: Item::Open(session.id),
                title: session.to.name.clone(),
                detail: match session.place.is_tab() {
                    true => host.clone(),
                    false => format!("{host} · Session window"),
                },
            });
        }
    }
    let mut saved: Vec<_> = library
        .map(|l| l.connections.iter().filter(|c| hit(&[&c.name, &c.host])).collect())
        .unwrap_or_default();
    saved.sort_by_key(|c| c.name.to_lowercase());
    for connection in &saved {
        let credential = library
            .and_then(|l| l.credential_of(connection))
            .map_or("Ask each time", |c| c.name.as_str());
        rows.push(Row {
            section: Section::Saved,
            item: Item::Saved(connection.id.clone()),
            title: connection.name.clone(),
            detail: format!("{} · {credential}", host_text(connection)),
        });
    }
    if !typed.is_empty() {
        rows.push(Row {
            section: Section::NewHost,
            item: Item::NewHost(typed.to_owned()),
            title: format!("Connect to \u{201c}{typed}\u{201d}"),
            detail: String::new(),
        });
        for connection in &saved {
            rows.push(Row {
                section: Section::Manage,
                item: Item::Edit(connection.id.clone()),
                title: format!("Edit {}…", connection.name),
                detail: String::new(),
            });
        }
    }
    if library.is_some() {
        for (item, title) in [
            (Item::Credentials, "Credentials…"),
            (Item::Certificates, "Certificates…"),
        ] {
            if hit(&[title]) {
                rows.push(Row {
                    section: Section::Manage,
                    item,
                    title: title.into(),
                    detail: String::new(),
                });
            }
        }
    }
    rows
}

impl Bezel {
    fn palette_rows(&self) -> Vec<Row> {
        rows(&self.here.palette.query.text, &self.sessions, self.library.ready())
    }

    /// ⌘K: the floating palette over this window, or the Main window's inline field when it
    /// shows no Tab.
    pub fn open_palette(&mut self, tx: &mut Tx) {
        let inline = self.here.window == Window::Main && self.active.is_none();
        let palette = &mut self.here.palette;
        palette.open = !inline;
        palette.query = TextEdit::default();
        palette.query.focused = true;
        palette.selected = 0;
        palette.error = None;
        tx.focus("palette-input", false);
    }

    pub fn close_palette(&mut self) {
        let palette = &mut self.here.palette;
        palette.open = false;
        palette.query = TextEdit::default();
        palette.selected = 0;
        palette.error = None;
    }

    /// Keys in the palette's field: arrows move the selection, Return and its modifiers choose.
    pub fn palette_key(&mut self, event: &KeyEvent) -> bool {
        if !self.palette_shows_rows() {
            return false;
        }
        let count = self.palette_rows().len();
        let palette = &mut self.here.palette;
        match event.key {
            Key::Down if count > 0 => palette.selected = (palette.selected + 1) % count,
            Key::Up if count > 0 => palette.selected = (palette.selected + count - 1) % count,
            Key::Enter => self.choose(self.here.palette.selected, How::of(event)),
            _ => return false,
        }
        true
    }

    /// The floating palette always lists its matches; the inline field only once something is
    /// typed, and until then has nothing to choose.
    fn palette_shows_rows(&self) -> bool {
        self.here.palette.open || !self.here.palette.query.text.trim().is_empty()
    }

    pub fn palette_edited(&mut self) {
        self.here.palette.selected = 0;
        self.here.palette.error = None;
    }

    /// Acts on the row at `index`.
    pub fn choose(&mut self, index: usize, how: How) {
        let Some(row) = self.palette_rows().into_iter().nth(index) else {
            return;
        };
        if let Item::NewHost(typed) = &row.item
            && let Err(e) = Address::parse(typed)
        {
            self.here.palette.error = Some(e.to_string());
            return;
        }
        self.close_palette();
        match (row.item, how) {
            (Item::Open(id), How::Return) => self.reveal(id),
            (Item::Open(id), How::Command) => match self.session(id).map(|s| s.place.is_tab()) {
                Some(true) => self.move_to_window(id, None),
                _ => self.reveal(id),
            },
            (Item::Open(id), How::Option) => self.open_another(id),
            (Item::Saved(id), How::Return) => self.open(&id, Opening::Tab, false),
            (Item::Saved(id), How::Command) => self.open(&id, Opening::Window, false),
            (Item::Saved(id), How::Option) | (Item::Edit(id), _) => {
                if self.ready_to_manage() {
                    self.edit_connection(&id);
                }
            }
            (Item::NewHost(typed), how) => {
                let Ok(address) = Address::parse(&typed) else { return };
                match how {
                    How::Return => {
                        let id = self.open_host(address, Opening::Tab);
                        self.reveal(id);
                    }
                    How::Command => {
                        self.open_host(address, Opening::Window);
                    }
                    How::Option if self.ready_to_manage() => {
                        self.new_connection(None);
                        if let Some(Sheet::Connection(form)) = &mut self.sheet {
                            form.host = TextEdit::new(typed);
                        }
                    }
                    How::Option => {}
                }
            }
            (Item::Credentials, _) => {
                if self.ready_to_manage() {
                    self.open_credentials(None);
                }
            }
            (Item::Certificates, _) => {
                if self.ready_to_manage() {
                    self.open_certificates();
                }
            }
        }
    }

    /// Another Session of what the Session `id` is for, in a new Tab.
    fn open_another(&mut self, id: SessionId) {
        let Some(to) = self.session(id).map(|s| s.to.clone()) else {
            return;
        };
        match to.connection {
            Some(connection) => self.open(&connection, Opening::Tab, true),
            None => {
                let id = self.open_host(to.address, Opening::Tab);
                self.reveal(id);
            }
        }
    }
}

fn input(s: &Bezel, k: &Kit, large: bool) -> El {
    let mut input = TextInput::new(&s.here.palette.query)
        .label("Connect or search")
        .placeholder("Type a host name to connect")
        .leading_icon(k.glyph.search)
        .width(None)
        .invalid(s.here.palette.error.is_some())
        .on_edit(|s: &mut Bezel| &mut s.here.palette.query)
        .on_change(|s: &mut Bezel, _: &mut Tx| s.palette_edited())
        .on_key(|s: &mut Bezel, e: &KeyEvent, _: &mut Tx| s.palette_key(e));
    if large {
        input = input.large();
    }
    input.build(k).key("palette-input")
}

/// The matches under the field, grouped by section, the selection highlighted.
fn list(s: &Bezel, k: &Kit) -> El {
    let rows = s.palette_rows();
    let mut list = col().key("palette-list").role("listbox").w_full().gap(2.0);
    let mut section = None;
    for (index, row) in rows.iter().enumerate() {
        if section != Some(row.section) {
            section = Some(row.section);
            list = list.child(
                text(row.section.title())
                    .font(k.ty.small_medium)
                    .color(k.c.text_faint)
                    .px(8.0)
                    .pt(if index == 0 { 0.0 } else { 8.0 })
                    .pb(2.0),
            );
        }
        list = list.child(row_el(row, index, index == s.here.palette.selected, k));
    }
    if rows.is_empty() {
        list = list.child(text("No matches").font(k.ty.ui).color(k.c.text_faint).px(8.0).py(6.0));
    }
    list
}

fn row_el(entry: &Row, index: usize, selected: bool, k: &Kit) -> El {
    let glyph = match entry.item {
        Item::Open(_) | Item::Saved(_) => &icons::MONITOR,
        Item::NewHost(_) => &icons::ARROW_RIGHT,
        Item::Edit(_) => &icons::PENCIL_LINE,
        Item::Credentials => &icons::LOCK,
        Item::Certificates => &icons::SHIELD,
    };
    let mut line = row()
        .key(format!("palette-row-{index}"))
        .role("option")
        .label(entry.title.clone())
        .w_full()
        .h(32.0)
        .px(8.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.md)
        .child(rune::icon(glyph, 16.0).color(k.c.icon_muted))
        .child(
            text(entry.title.clone())
                .font(k.ty.ui_medium)
                .color(k.c.text_base)
                .shrink()
                .truncate(),
        )
        .child(
            text(entry.detail.clone())
                .font(k.ty.ui)
                .color(k.c.text_muted)
                .grow(1.0)
                .shrink()
                .truncate(),
        )
        .on_press_tx(move |s: &mut Bezel, tx: &mut Tx| {
            let how = match tx.mods() {
                m if m.cmd => How::Command,
                m if m.alt => How::Option,
                _ => How::Return,
            };
            s.choose(index, how);
        });
    line = k.interactive(line);
    if selected {
        line = line.fill(k.c.pressed).child(Keybind::new(&["↵"]).build(k));
    }
    line
}

fn hints(k: &Kit) -> El {
    let hint = |keys: &[&str], what: &str| {
        row()
            .gap(6.0)
            .center_y()
            .child(Keybind::new(keys).build(k))
            .child(text(what.to_owned()).font(k.ty.small).color(k.c.text_muted))
    };
    row()
        .w_full()
        .gap(16.0)
        .px(12.0)
        .py(8.0)
        .child(hint(&["↵"], "Open"))
        .child(hint(&["⌘", "↵"], "In new window"))
        .child(hint(&["⌥", "↵"], "Edit or open another"))
        .child(el().grow(1.0))
        .child(hint(&["esc"], "Close"))
}

/// The floating palette over the window, with a scrim.
pub fn floating(s: &Bezel, k: &Kit, layer: &mut Vec<El>) {
    if !s.here.palette.open {
        return;
    }
    let mut body = col().w_full().gap(8.0).padding(12.0).child(input(s, k, true));
    if let Some(error) = &s.here.palette.error {
        body = body.child(text(error.clone()).font(k.ty.ui).color(k.c.danger.fg).px(4.0));
    }
    body = body.child(
        col()
            .w_full()
            .max_h(360.0)
            .min_h(0.0)
            .shrink()
            .scroll(Default::default())
            .child(list(s, k)),
    );
    let panel = k
        .elevate(
            col()
                .key("palette")
                .role("dialog")
                .label("Command palette")
                .w_full()
                .max_w(560.0)
                .shrink()
                .min_h(0.0)
                .radius(k.radius.lg)
                .fill(k.c.layer_01)
                .child(body)
                .child(el().w_full().h(1.0).fill(k.c.border_weak))
                .child(hints(k))
                .token_motion(k.motion.overlay),
            &k.elevation.overlay,
        )
        .modal(true, |s: &mut Bezel| s.close_palette());
    layer.push(
        el().key("palette-scrim")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .fill(k.c.scrim)
            .token_motion(k.motion.overlay)
            .a11y_hidden(),
    );
    layer.push(
        col()
            .key("palette-layer")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .px(16.0)
            .pt(72.0)
            .items_center()
            .child(panel),
    );
}

/// The empty Main window's connect box: the palette's field inline, its matches under it once
/// something is typed.
pub fn inline(s: &Bezel, k: &Kit) -> El {
    let mut column = col().key("connect-box").w_full().gap(8.0).child(input(s, k, true));
    if let Some(error) = &s.here.palette.error {
        column = column.child(text(error.clone()).font(k.ty.ui).color(k.c.danger.fg));
    }
    if !s.here.palette.query.text.trim().is_empty() {
        column = column.child(
            col()
                .w_full()
                .padding(4.0)
                .radius(k.radius.md)
                .fill(k.c.layer_01)
                .border(1.0, k.c.border_weak)
                .max_h(320.0)
                .min_h(0.0)
                .scroll(Default::default())
                .child(list(s, k)),
        );
    }
    column
}

#[cfg(test)]
mod tests {
    use bezel_core::library::{Connection, Credential};

    use super::*;
    use crate::app::{Destination, Phase, fixture_session};

    fn library() -> Snapshot {
        let connection = |id: &str, name: &str, credential: Option<&str>| Connection {
            id: id.into(),
            name: name.into(),
            host: format!("{name}.corp.lan"),
            port: 3389,
            credential: credential.map(Into::into),
            group: None,
        };
        Snapshot {
            credentials: vec![Credential {
                id: "c_admin".into(),
                name: "Domain admin".into(),
                username: r"CORP\admin".into(),
                password_updated: None,
            }],
            connections: vec![
                connection("n_fs01", "fs01", None),
                connection("n_dc02", "dc02", Some("c_admin")),
                connection("n_dc01", "dc01", Some("c_admin")),
            ],
            ..Snapshot::default()
        }
    }

    fn titles(rows: &[Row]) -> Vec<(Section, &str)> {
        rows.iter().map(|r| (r.section, r.title.as_str())).collect()
    }

    #[test]
    fn typing_matches_open_saved_new_host_then_manage() {
        let library = library();
        let open = fixture_session(
            1,
            Destination::connection(&library.connections[2]),
            Phase::Connecting,
            None,
        );
        let rows = rows("dc", &[open], Some(&library));
        assert_eq!(
            titles(&rows),
            vec![
                (Section::Open, "dc01"),
                (Section::Saved, "dc01"),
                (Section::Saved, "dc02"),
                (Section::NewHost, "Connect to \u{201c}dc\u{201d}"),
                (Section::Manage, "Edit dc01…"),
                (Section::Manage, "Edit dc02…"),
            ]
        );
        assert_eq!(rows[1].detail, "dc01.corp.lan · Domain admin");
    }

    #[test]
    fn a_host_matches_too_and_nothing_typed_lists_every_connection() {
        let library = library();
        let by_host = rows("FS01.CORP", &[], Some(&library));
        assert_eq!(by_host[0].item, Item::Saved("n_fs01".into()));
        assert_eq!(by_host[0].detail, "fs01.corp.lan · Ask each time");
        let all = rows("", &[], Some(&library));
        assert_eq!(
            titles(&all),
            vec![
                (Section::Saved, "dc01"),
                (Section::Saved, "dc02"),
                (Section::Saved, "fs01"),
                (Section::Manage, "Credentials…"),
                (Section::Manage, "Certificates…"),
            ]
        );
    }

    #[test]
    fn before_the_library_loads_only_open_sessions_and_new_hosts_match() {
        let rows = rows("10.0.0.5", &[], None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].item, Item::NewHost("10.0.0.5".into()));
    }
}
