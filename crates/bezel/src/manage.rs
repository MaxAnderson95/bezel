//! Managing the Library from the Main window: the Connection, Group, Credentials, Certificates
//! and Settings sheets, confirmations, context-menu actions and dragging in the sidebar.

use std::rc::Rc;

use bezel_core::address::Address;
use bezel_core::library::{
    Change, Connection, ConnectionId, Credential, CredentialId, Group, GroupId, Setting, Settings,
};
use bezel_core::secrets::Secret;
use bezel_core::trust::Fingerprint;
use rune::{Drag, Phase, TextEdit, Tx};

use crate::app::{Bezel, Destination, SessionId, today};
use crate::store::RequestId;

pub enum Sheet {
    Connection(ConnectionForm),
    Group(GroupForm),
    Credentials(Credentials),
    Certificates,
    Settings(SettingsSheet),
}

/// Each setting is saved as soon as it is changed. The sheet shows the Library's settings with
/// the changes still being saved on top, so a refused one goes back to what is on disk.
#[derive(Default)]
pub struct SettingsSheet {
    saving: Vec<(RequestId, Setting)>,
    pub error: Option<String>,
}

impl SettingsSheet {
    pub fn shown(&self, saved: Settings) -> Settings {
        self.saving
            .iter()
            .fold(saved, |settings, (_, setting)| settings.with(*setting))
    }
}

pub struct ConnectionForm {
    /// The Connection as it was when the sheet opened; `None` for a new one.
    pub editing: Option<Connection>,
    pub name: TextEdit,
    pub host: TextEdit,
    pub credential: Option<CredentialId>,
    pub group: Option<GroupId>,
    pub error: Option<String>,
    saving: Option<RequestId>,
    /// The New host Session this Connection is saved from.
    session: Option<Adopt>,
}

/// A New host Session waiting for the Connection saved from it.
struct Adopt {
    session: SessionId,
    /// The Session's address when the sheet opened.
    address: Address,
    /// The Connection being saved, once Save is pressed.
    connection: Option<ConnectionId>,
}

impl ConnectionForm {
    pub fn saving(&self) -> bool {
        self.saving.is_some()
    }
}

pub struct GroupForm {
    /// The Group as it was when the sheet opened; `None` for a new one.
    pub renaming: Option<Group>,
    pub name: TextEdit,
}

pub struct Credentials {
    pub search: TextEdit,
    pub selected: Selected,
    pub name: TextEdit,
    pub username: TextEdit,
    pub password: TextEdit,
    pub error: Option<String>,
    saving: Option<RequestId>,
}

impl Credentials {
    pub fn saving(&self) -> bool {
        self.saving.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selected {
    Nothing,
    /// A saved Credential, as it was when its fields were filled in.
    Existing(Credential),
    /// A Credential being added, with the ID it will have.
    New(CredentialId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delete {
    Connection(ConnectionId),
    Group(GroupId),
    Credential(CredentialId),
}

/// A sidebar row or Group carried by the pointer, `dx`, `dy` from where it was pressed.
#[derive(Clone, Debug, PartialEq)]
pub struct Dragging {
    pub item: Dragged,
    pub dx: f32,
    pub dy: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dragged {
    Connection(ConnectionId),
    Group(GroupId),
}

/// Where a drop can land: the sidebar section's element key and its Group (`None` for
/// Ungrouped).
pub type DropTargets = Rc<Vec<(String, Option<GroupId>)>>;

/// Points the pointer travels before a press becomes a drag.
const SLOP: f32 = 4.0;

/// The name a Connection gets when its Name is left empty: the host's first label (`dc01` for
/// `dc01.corp.lan`), or the whole address for an IP.
pub fn default_name(host: &str) -> String {
    let host = host.trim();
    if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
        return host.into();
    }
    host.split(['.', ':']).next().unwrap_or(host).into()
}

/// A Connection's address as its Host field shows it: the port only when it isn't 3389.
pub fn host_text(connection: &Connection) -> String {
    address_text(&connection.address())
}

pub fn address_text(address: &Address) -> String {
    match (address.port, address.host.contains(':')) {
        (bezel_core::address::DEFAULT_PORT, _) => address.host.clone(),
        (port, true) => format!("[{}]:{port}", address.host),
        (port, false) => format!("{}:{port}", address.host),
    }
}

/// A form field follows the Library: when the saved value moved from `baseline`, the value the
/// form was opened from, the field shows the saved one. A field the person changed keeps their
/// value until the Library holds something else for it, either because their save committed it
/// or because it was changed elsewhere (which refused their save).
fn follow<T: Clone + PartialEq>(field: &mut T, baseline: &T, now: &T) {
    if baseline != now {
        *field = now.clone();
    }
}

fn follow_text(field: &mut TextEdit, baseline: &str, now: &str) {
    if baseline != now {
        let focused = field.focused;
        *field = TextEdit::new(now);
        field.focused = focused;
    }
}

fn focused(text: impl Into<String>) -> TextEdit {
    let mut edit = TextEdit::new(text);
    edit.focused = true;
    edit
}

impl Bezel {
    /// Brings the Main window forward for a Library action asked for from any window. False when
    /// a sheet or confirmation is already open there: it stays as it is, draft and pending save
    /// included, for the person to finish or close first.
    pub(crate) fn ready_to_manage(&mut self) -> bool {
        self.show_main();
        self.sheet.is_none() && self.deleting.is_none()
    }

    pub fn close_sheet(&mut self) {
        self.sheet = None;
    }

    pub fn new_connection(&mut self, group: Option<GroupId>) {
        self.sheet = Some(Sheet::Connection(ConnectionForm {
            editing: None,
            name: TextEdit::default(),
            host: focused(""),
            credential: None,
            group,
            error: None,
            saving: None,
            session: None,
        }));
    }

    /// "Save as connection" for a New host: the New connection sheet with its address, and
    /// once saved, the Session is that Connection's.
    pub fn save_as_connection(&mut self, session: SessionId) {
        let Some(address) = self
            .session(session)
            .filter(|s| s.to.connection.is_none())
            .map(|s| s.to.address.clone())
        else {
            return;
        };
        if !self.ready_to_manage() {
            return;
        }
        self.new_connection(None);
        if let Some(Sheet::Connection(form)) = &mut self.sheet {
            form.host = focused(address_text(&address));
            form.session = Some(Adopt {
                session,
                address,
                connection: None,
            });
        }
    }

    pub fn edit_connection(&mut self, id: &ConnectionId) {
        let Some(connection) = self.library.ready().and_then(|s| s.connection(id)) else {
            return;
        };
        self.sheet = Some(Sheet::Connection(ConnectionForm {
            editing: Some(connection.clone()),
            name: focused(connection.name.clone()),
            host: TextEdit::new(host_text(connection)),
            credential: connection.credential.clone(),
            group: connection.group.clone(),
            error: None,
            saving: None,
            session: None,
        }));
    }

    pub fn set_form_credential(&mut self, credential: Option<CredentialId>) {
        if let Some(Sheet::Connection(form)) = &mut self.sheet {
            form.credential = credential;
        }
    }

    pub fn set_form_group(&mut self, group: Option<GroupId>) {
        if let Some(Sheet::Connection(form)) = &mut self.sheet {
            form.group = group;
        }
    }

    pub fn save_connection(&mut self) {
        let Some(Sheet::Connection(form)) = &mut self.sheet else {
            return;
        };
        if form.saving.is_some() {
            return;
        }
        let address = match Address::parse(&form.host.text) {
            Ok(address) => address,
            Err(e) => return form.error = Some(e.to_string()),
        };
        let name = match form.name.text.trim() {
            "" => default_name(&address.host),
            name => name.to_owned(),
        };
        let connection = Connection {
            id: form
                .editing
                .as_ref()
                .map_or_else(ConnectionId::random, |c| c.id.clone()),
            name,
            host: address.host,
            port: address.port,
            credential: form.credential.clone(),
            group: form.group.clone(),
        };
        if let Some(adopt) = &mut form.session {
            adopt.connection = Some(connection.id.clone());
        }
        let change = match form.editing.clone() {
            Some(original) => Change::EditConnection {
                original,
                edited: connection,
            },
            None => Change::AddConnection(connection),
        };
        form.error = None;
        let id = self.apply(change);
        if let Some(Sheet::Connection(form)) = &mut self.sheet {
            form.saving = id;
        }
    }

    pub fn duplicate_connection(&mut self, id: &ConnectionId) {
        let Some(connection) = self.library.ready().and_then(|s| s.connection(id)) else {
            return;
        };
        let copy = Connection {
            id: ConnectionId::random(),
            name: format!("{} copy", connection.name),
            ..connection.clone()
        };
        self.apply(Change::AddConnection(copy));
    }

    pub fn move_connection(&mut self, connection: ConnectionId, group: Option<GroupId>) {
        let current = self
            .library
            .ready()
            .and_then(|s| s.connection(&connection))
            .map(|c| &c.group);
        if current.is_some_and(|current| *current != group) {
            self.apply(Change::MoveConnection { connection, group });
        }
    }

    pub fn new_group(&mut self) {
        self.sheet = Some(Sheet::Group(GroupForm {
            renaming: None,
            name: focused(""),
        }));
    }

    pub fn rename_group(&mut self, id: &GroupId) {
        let Some(group) = self.library.ready().and_then(|s| s.group(id)) else {
            return;
        };
        self.sheet = Some(Sheet::Group(GroupForm {
            renaming: Some(group.clone()),
            name: focused(group.name.clone()),
        }));
    }

    pub fn save_group(&mut self) {
        let Some(Sheet::Group(form)) = &self.sheet else { return };
        let name = form.name.text.trim().to_owned();
        if name.is_empty() {
            return;
        }
        let change = match &form.renaming {
            Some(group) => Change::RenameGroup {
                group: group.id.clone(),
                from: group.name.clone(),
                name,
            },
            None => Change::AddGroup(Group {
                id: GroupId::random(),
                name,
            }),
        };
        self.sheet = None;
        self.apply(change);
    }

    /// Moves the Group one place up (`-1`) or down (`1`) in the sidebar.
    pub fn step_group(&mut self, id: &GroupId, step: isize) {
        let Some(groups) = self.library.ready().map(|s| &s.groups) else {
            return;
        };
        let Some(from) = groups.iter().position(|g| &g.id == id) else {
            return;
        };
        let Some(to) = from.checked_add_signed(step).filter(|to| *to < groups.len()) else {
            return;
        };
        self.move_group_to(id, from, to);
    }

    /// Moves the Group from place `from` to place `to` among the Groups as they are now.
    fn move_group_to(&mut self, id: &GroupId, from: usize, to: usize) {
        let Some(groups) = self.library.ready().map(|s| &s.groups) else {
            return;
        };
        let before = match to.cmp(&from) {
            std::cmp::Ordering::Equal => return,
            std::cmp::Ordering::Less => groups.get(to),
            std::cmp::Ordering::Greater => groups.get(to + 1),
        };
        let change = Change::MoveGroup {
            group: id.clone(),
            before: before.map(|g| g.id.clone()),
        };
        self.apply(change);
    }

    pub fn ask_delete(&mut self, delete: Delete) {
        self.deleting = Some(delete);
    }

    pub fn cancel_delete(&mut self) {
        self.deleting = None;
    }

    pub fn delete(&mut self) {
        let Some(delete) = self.deleting.take() else { return };
        let change = match delete {
            Delete::Connection(id) => Change::DeleteConnection(id),
            Delete::Group(id) => Change::DeleteGroup(id),
            Delete::Credential(id) => {
                if let Some(Sheet::Credentials(sheet)) = &mut self.sheet
                    && matches!(&sheet.selected, Selected::Existing(c) if c.id == id)
                {
                    sheet.selected = Selected::Nothing;
                }
                Change::DeleteCredential(id)
            }
        };
        self.apply(change);
    }

    pub fn open_credentials(&mut self, credential: Option<CredentialId>) {
        let mut sheet = Credentials {
            search: TextEdit::default(),
            selected: Selected::Nothing,
            name: TextEdit::default(),
            username: TextEdit::default(),
            password: TextEdit::default().secure(),
            error: None,
            saving: None,
        };
        let first = self
            .library
            .ready()
            .and_then(|s| s.credentials.first())
            .map(|c| c.id.clone());
        if let Some(id) = credential.or(first) {
            self.fill_credential(&mut sheet, &id);
        }
        self.sheet = Some(Sheet::Credentials(sheet));
    }

    fn fill_credential(&self, sheet: &mut Credentials, id: &CredentialId) {
        let Some(credential) = self.library.ready().and_then(|s| s.credential(id)) else {
            return;
        };
        sheet.selected = Selected::Existing(credential.clone());
        sheet.name = TextEdit::new(credential.name.clone());
        sheet.username = TextEdit::new(credential.username.clone());
        sheet.password = TextEdit::default().secure();
        sheet.error = None;
    }

    pub fn select_credential(&mut self, id: &CredentialId) {
        let Some(Sheet::Credentials(mut sheet)) = self.sheet.take() else {
            return;
        };
        if !sheet.saving() {
            self.fill_credential(&mut sheet, id);
        }
        self.sheet = Some(Sheet::Credentials(sheet));
    }

    pub fn new_credential(&mut self) {
        let Some(Sheet::Credentials(sheet)) = &mut self.sheet else {
            return;
        };
        if sheet.saving() {
            return;
        }
        sheet.selected = Selected::New(CredentialId::random());
        sheet.name = focused("");
        sheet.username = TextEdit::default();
        sheet.password = TextEdit::default().secure();
        sheet.error = None;
    }

    /// Saves the selected Credential's name and username, and its password when one was typed:
    /// one Keychain item that every Connection using the Credential reads.
    pub fn save_credential(&mut self) {
        let Some(Sheet::Credentials(sheet)) = &mut self.sheet else {
            return;
        };
        if sheet.saving.is_some() {
            return;
        }
        let typed = (!sheet.password.text.is_empty()).then(|| Secret::from(sheet.password.text.clone()));
        let (name, username) = (sheet.name.text.trim().to_owned(), sheet.username.text.trim().to_owned());
        let change = match &sheet.selected {
            Selected::Nothing => return,
            Selected::New(id) => {
                let Some(password) = typed else {
                    return sheet.error = Some("Enter the password".into());
                };
                Change::AddCredential {
                    credential: Credential {
                        id: id.clone(),
                        name,
                        username,
                        password_updated: Some(today()),
                    },
                    password,
                }
            }
            Selected::Existing(original) => {
                let edited = Credential {
                    name,
                    username,
                    ..original.clone()
                };
                if edited == *original && typed.is_none() {
                    return;
                }
                Change::EditCredential {
                    original: original.clone(),
                    edited,
                    password: typed,
                    today: today(),
                }
            }
        };
        sheet.error = None;
        let id = self.apply(change);
        if let Some(Sheet::Credentials(sheet)) = &mut self.sheet {
            sheet.saving = id;
        }
    }

    pub fn open_certificates(&mut self) {
        self.sheet = Some(Sheet::Certificates);
    }

    /// ⌘, from any window, the sidebar or the palette.
    pub fn open_settings(&mut self) {
        if self.library.ready().is_some() && self.ready_to_manage() {
            self.sheet = Some(Sheet::Settings(SettingsSheet::default()));
        }
    }

    pub fn set_setting(&mut self, setting: Setting) {
        if !matches!(self.sheet, Some(Sheet::Settings(_))) {
            return;
        }
        let request = self.apply(Change::Setting(setting));
        if let (Some(Sheet::Settings(sheet)), Some(request)) = (&mut self.sheet, request) {
            sheet.error = None;
            sheet.saving.push((request, setting));
        }
    }

    /// Forgets the Trusted certificate the list showed, unless another has replaced it since.
    pub fn forget_certificate(&mut self, host: String, port: u16, sha256: Fingerprint) {
        self.apply(Change::ForgetCertificate { host, port, sha256 });
    }

    /// Brings every open form up to the Library after any reply from it. Each form keeps the
    /// item as it was opened (its baseline); fields follow the Library where it moved from the
    /// baseline (see `follow`), and a password typed for one account is cleared when the
    /// Credential's username changes, so it never reaches another account.
    pub(crate) fn follow_library(&mut self) {
        self.follow_retries();
        let Some(snapshot) = self.library.ready() else { return };
        match &mut self.sheet {
            Some(Sheet::Connection(form)) => {
                let Some(baseline) = &form.editing else { return };
                let Some(now) = snapshot.connection(&baseline.id) else {
                    return;
                };
                follow_text(&mut form.name, &baseline.name, &now.name);
                follow_text(&mut form.host, &host_text(baseline), &host_text(now));
                follow(&mut form.credential, &baseline.credential, &now.credential);
                follow(&mut form.group, &baseline.group, &now.group);
                form.editing = Some(now.clone());
            }
            Some(Sheet::Group(form)) => {
                let Some(baseline) = &form.renaming else { return };
                let Some(now) = snapshot.group(&baseline.id) else {
                    return;
                };
                follow_text(&mut form.name, &baseline.name, &now.name);
                form.renaming = Some(now.clone());
            }
            Some(Sheet::Credentials(sheet)) => {
                let Selected::Existing(baseline) = &sheet.selected else {
                    return;
                };
                let Some(now) = snapshot.credential(&baseline.id) else {
                    return;
                };
                follow_text(&mut sheet.name, &baseline.name, &now.name);
                follow_text(&mut sheet.username, &baseline.username, &now.username);
                if baseline.username != now.username {
                    sheet.password = TextEdit::default().secure();
                }
                sheet.selected = Selected::Existing(now.clone());
            }
            Some(Sheet::Certificates | Sheet::Settings(_)) | None => {}
        }
    }

    /// Takes the reply to a sheet's save, after `follow_library`. False when no sheet was
    /// waiting for it.
    pub(crate) fn on_sheet_saved(&mut self, id: RequestId, error: Option<&str>) -> bool {
        match &mut self.sheet {
            Some(Sheet::Connection(form)) if form.saving == Some(id) => {
                form.saving = None;
                match error {
                    None => {
                        let adopt = form.session.take();
                        self.sheet = None;
                        if let Some(adopt) = adopt {
                            self.adopt(adopt);
                        }
                    }
                    Some(e) => form.error = Some(e.into()),
                }
            }
            Some(Sheet::Settings(sheet)) if sheet.saving.iter().any(|(saving, _)| *saving == id) => {
                sheet.saving.retain(|(saving, _)| *saving != id);
                if let Some(e) = error {
                    sheet.error = Some(format!("That setting was not saved. {e}"));
                }
            }
            Some(Sheet::Credentials(sheet)) if sheet.saving == Some(id) => {
                sheet.saving = None;
                sheet.error = error.map(Into::into);
                match (&sheet.selected, error) {
                    (Selected::Existing(_), None) => sheet.password.clear(),
                    (Selected::New(id), None) => {
                        if let Some(now) = self.library.ready().and_then(|s| s.credential(id)) {
                            sheet.selected = Selected::Existing(now.clone());
                            sheet.password.clear();
                        }
                    }
                    // Refused, perhaps because its ID was taken: the next try is a new
                    // Credential with an ID of its own.
                    (Selected::New(_), Some(_)) => sheet.selected = Selected::New(CredentialId::random()),
                    (Selected::Existing(_) | Selected::Nothing, Some(_)) | (Selected::Nothing, None) => {}
                }
            }
            _ => return false,
        }
        true
    }

    /// The New host Session becomes the Session of the Connection just saved from it, if that
    /// Connection reaches the address the Session does.
    fn adopt(&mut self, adopt: Adopt) {
        let saved = adopt
            .connection
            .and_then(|id| self.library.ready()?.connection(&id))
            .filter(|c| c.address() == adopt.address)
            .map(Destination::connection);
        let Some(saved) = saved else { return };
        if let Some(session) = self
            .session_mut(adopt.session)
            .filter(|s| s.to.connection.is_none() && s.to.address == adopt.address)
        {
            session.to = saved;
        }
    }

    /// A sidebar row or Group header dragged: it follows the pointer, and a drop on a section
    /// moves the Connection into that Group, or the Group to that place.
    pub fn drag(&mut self, item: Dragged, d: &Drag, targets: &DropTargets, tx: &mut Tx) {
        match d.phase {
            Phase::Down => self.dragging = None,
            Phase::Move => {
                if self.dragging.is_some() || d.dx.abs().max(d.dy.abs()) >= SLOP {
                    self.dragging = Some(Dragging {
                        item,
                        dx: d.dx,
                        dy: d.dy,
                    });
                }
            }
            Phase::Up => {
                let dropped = self.dragging.take().is_some_and(|d| d.item == item);
                if !dropped || d.cancelled() {
                    return;
                }
                let [x, y] = d.window_point();
                for (key, group) in targets.iter() {
                    let (item, group) = (item.clone(), group.clone());
                    tx.read_layout(key.clone(), move |s: &mut Bezel, rect, _| {
                        if rect.is_some_and(|[rx, ry, w, h]| x >= rx && x < rx + w && y >= ry && y < ry + h) {
                            s.drop_on(item, group);
                        }
                    });
                }
            }
        }
    }

    fn drop_on(&mut self, item: Dragged, group: Option<GroupId>) {
        match item {
            Dragged::Connection(connection) => self.move_connection(connection, group),
            Dragged::Group(id) => {
                let Some(groups) = self.library.ready().map(|s| &s.groups) else {
                    return;
                };
                let Some(from) = groups.iter().position(|g| g.id == id) else {
                    return;
                };
                // Ungrouped sits below every Group, so a drop there makes the Group last.
                let to = match group {
                    Some(target) => groups.iter().position(|g| g.id == target),
                    None => groups.len().checked_sub(1),
                };
                if let Some(to) = to {
                    self.move_group_to(&id, from, to);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_is_named_after_its_hosts_first_label() {
        assert_eq!(default_name("dc01.corp.lan"), "dc01");
        assert_eq!(default_name(" FS01 "), "FS01");
        assert_eq!(default_name("10.0.0.5"), "10.0.0.5");
        assert_eq!(default_name("fe80::1"), "fe80::1");
    }

    #[test]
    fn the_host_field_shows_a_port_other_than_3389() {
        let mut connection = Connection {
            id: "n_dc01".into(),
            name: "dc01".into(),
            host: "dc01.corp.lan".into(),
            port: 3389,
            credential: None,
            group: None,
        };
        assert_eq!(host_text(&connection), "dc01.corp.lan");
        connection.port = 3390;
        assert_eq!(host_text(&connection), "dc01.corp.lan:3390");
        connection.host = "fe80::1".into();
        assert_eq!(host_text(&connection), "[fe80::1]:3390");
        assert_eq!(Address::parse(&host_text(&connection)).unwrap().port, 3390);
    }
}
