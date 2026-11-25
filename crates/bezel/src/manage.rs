//! Managing the Library from the Main window: the Connection, Group, Credentials and
//! Certificates sheets, confirmations, context-menu actions and dragging in the sidebar.

use std::rc::Rc;

use bezel_core::address::Address;
use bezel_core::library::{Change, Connection, ConnectionId, Credential, CredentialId, Group, GroupId};
use bezel_core::secrets::Secret;
use rune::{Drag, Phase, TextEdit, Tx};

use crate::app::{Bezel, today};
use crate::store::RequestId;

pub enum Sheet {
    Connection(ConnectionForm),
    Group(GroupForm),
    Credentials(Credentials),
    Certificates,
}

pub struct ConnectionForm {
    /// `None` for a new Connection.
    pub editing: Option<ConnectionId>,
    pub name: TextEdit,
    pub host: TextEdit,
    pub credential: Option<CredentialId>,
    pub group: Option<GroupId>,
    pub error: Option<String>,
    saving: Option<RequestId>,
}

impl ConnectionForm {
    pub fn saving(&self) -> bool {
        self.saving.is_some()
    }
}

pub struct GroupForm {
    /// `None` for a new Group.
    pub renaming: Option<GroupId>,
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
    Existing(CredentialId),
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
    match (connection.port, connection.host.contains(':')) {
        (bezel_core::address::DEFAULT_PORT, _) => connection.host.clone(),
        (port, true) => format!("[{}]:{port}", connection.host),
        (port, false) => format!("{}:{port}", connection.host),
    }
}

fn focused(text: impl Into<String>) -> TextEdit {
    let mut edit = TextEdit::new(text);
    edit.focused = true;
    edit
}

impl Bezel {
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
        }));
    }

    pub fn edit_connection(&mut self, id: &ConnectionId) {
        let Some(connection) = self.library.ready().and_then(|s| s.connection(id)) else {
            return;
        };
        self.sheet = Some(Sheet::Connection(ConnectionForm {
            editing: Some(connection.id.clone()),
            name: focused(connection.name.clone()),
            host: TextEdit::new(host_text(connection)),
            credential: connection.credential.clone(),
            group: connection.group.clone(),
            error: None,
            saving: None,
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
            id: form.editing.clone().unwrap_or_else(ConnectionId::random),
            name,
            host: address.host,
            port: address.port,
            credential: form.credential.clone(),
            group: form.group.clone(),
        };
        let change = match form.editing {
            Some(_) => Change::EditConnection(connection),
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
            renaming: Some(group.id.clone()),
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
                group: group.clone(),
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
                    && sheet.selected == Selected::Existing(id.clone())
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
        sheet.selected = Selected::Existing(id.clone());
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
            Selected::Existing(id) => {
                let Some(saved) = self.library.ready().and_then(|s| s.credential(id)) else {
                    return;
                };
                let name = (name != saved.name).then_some(name);
                let username = (username != saved.username).then_some(username);
                if name.is_none() && username.is_none() && typed.is_none() {
                    return;
                }
                Change::EditCredential {
                    credential: id.clone(),
                    name,
                    username,
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

    pub fn forget_certificate(&mut self, host: String, port: u16) {
        self.apply(Change::ForgetCertificate { host, port });
    }

    /// Takes the reply to a sheet's save. False when no sheet was waiting for it.
    pub(crate) fn on_sheet_saved(&mut self, id: RequestId, error: Option<&str>) -> bool {
        match &mut self.sheet {
            Some(Sheet::Connection(form)) if form.saving == Some(id) => {
                form.saving = None;
                match error {
                    None => self.sheet = None,
                    Some(e) => form.error = Some(e.into()),
                }
            }
            Some(Sheet::Credentials(sheet)) if sheet.saving == Some(id) => {
                sheet.saving = None;
                match error {
                    None => {
                        if let Selected::New(id) = &sheet.selected {
                            sheet.selected = Selected::Existing(id.clone());
                        }
                        sheet.password.clear();
                        sheet.error = None;
                    }
                    Some(e) => sheet.error = Some(e.into()),
                }
            }
            _ => return false,
        }
        true
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
