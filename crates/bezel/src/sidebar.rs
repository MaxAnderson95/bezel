//! The Main window's sidebar: Groups in Max's order with their Connections alphabetically,
//! Ungrouped last, status dots, hover actions, context menus and dragging, and a foot with
//! Credentials, Certificates, Settings and +.

use std::rc::Rc;

use bezel_core::library::{ConnectionId, GroupId, Section, Snapshot};
use rune::{Drag, El, PointerPress, PopupItem, PopupMenu, Tx, col, el, row, text};
use rune_kit::shell::RowStatus;
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Keybind, Kit, Sidebar, SidebarHeader, SidebarRow,
};
use rune_theme_opencode::icons;

use crate::app::{Bezel, Dot, Library, Opening};
use crate::manage::{Delete, Dragged, DropTargets};

pub const WIDTH: f32 = 240.0;

pub fn sidebar(s: &Bezel, k: &Kit) -> El {
    let column = col().key("sidebar-column").w(WIDTH).h_full().min_h(0.0);
    let list = match &s.library {
        Library::Ready(snapshot) => connections(s, snapshot, k),
        Library::Loading => Sidebar::new(),
        Library::Failed(_) => Sidebar::new().child(faint(k, "Bezel can't read its data file")),
    };
    column.child(list.label("Connections").header(search(k)).footer(foot(s, k)).build(k))
}

/// "Connect or search": opens the command palette.
fn search(k: &Kit) -> El {
    let field = row()
        .key("sidebar-search")
        .role("button")
        .label("Connect or search")
        .w_full()
        .h(28.0)
        .px(8.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.md)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_weak)
        .hover_motion(rune::Anim::INSTANT, rune::Anim::INSTANT)
        .child(rune::icon(k.glyph.search, 14.0).color(k.c.icon_faint))
        .child(
            text("Connect or search")
                .font(k.ty.ui)
                .color(k.c.text_faint)
                .grow(1.0)
                .shrink()
                .truncate(),
        )
        .child(Keybind::new(&["⌘", "K"]).build(k))
        .on_press_tx(|s: &mut Bezel, tx: &mut Tx| s.open_palette(tx));
    k.interactive(field)
}

fn faint(k: &Kit, message: &str) -> El {
    row().w_full().px(8.0).py(6.0).child(
        text(message.to_owned())
            .font(k.ty.ui)
            .color(k.c.text_faint)
            .wrap()
            .w_full(),
    )
}

fn section_key(group: Option<&GroupId>) -> String {
    match group {
        Some(id) => format!("section-{id}"),
        None => "section-ungrouped".into(),
    }
}

fn connections(s: &Bezel, snapshot: &Snapshot, k: &Kit) -> Sidebar {
    let sections = snapshot.sections();
    if snapshot.connections.is_empty() && snapshot.groups.is_empty() {
        return Sidebar::new().child(faint(k, "No connections yet"));
    }
    let targets: DropTargets = Rc::new(
        sections
            .iter()
            .map(|s| {
                let group = s.group.map(|g| g.id.clone());
                (section_key(group.as_ref()), group)
            })
            .collect(),
    );
    let mut list = Sidebar::new();
    for (index, section) in sections.iter().enumerate() {
        list = list.child(section_el(s, snapshot, section, index, &targets, k));
    }
    list
}

fn section_el(
    s: &Bezel,
    snapshot: &Snapshot,
    section: &Section<'_>,
    index: usize,
    targets: &DropTargets,
    k: &Kit,
) -> El {
    let group = section.group.map(|g| g.id.clone());
    let title = section.group.map_or("Ungrouped", |g| g.name.as_str());
    let mut header = SidebarHeader::new(title).build(k);
    if let Some(id) = group.clone() {
        let count = snapshot.groups.len();
        let menu_id = id.clone();
        let targets = targets.clone();
        header = header
            .on_secondary_press(move |_: &mut Bezel, press: PointerPress, tx: &mut Tx| {
                group_menu(menu_id.clone(), index, count, press, tx)
            })
            .on_drag(move |s: &mut Bezel, d: &mut Drag, tx: &mut Tx| {
                s.drag(Dragged::Group(id.clone()), d, &targets, tx)
            });
    }
    let mut column = col()
        .key(section_key(group.as_ref()))
        .role("group")
        .label(title.to_owned())
        .w_full()
        .gap(2.0)
        .pb(8.0)
        .child(header);
    for connection in &section.connections {
        column = column.child(connection_row(
            s,
            snapshot,
            &connection.id,
            &connection.name,
            targets,
            k,
        ));
    }
    match (&s.dragging, group) {
        (Some(d), Some(group)) if d.item == Dragged::Group(group.clone()) => {
            k.elevate(column.offset(d.dx, d.dy).raise().fill(k.c.bg_deep), &k.elevation.raised)
        }
        _ => column,
    }
}

fn connection_row(s: &Bezel, snapshot: &Snapshot, id: &ConnectionId, name: &str, targets: &DropTargets, k: &Kit) -> El {
    let dot = s.dot(id);
    let (window, edit) = (id.clone(), id.clone());
    // Pointer only, as the kit's row close button: the list's arrows step through rows.
    let action = |glyph: &'static rune::Icon, label: &str, key: &'static str| {
        IconButton::new(glyph)
            .variant(IconButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .label(label.to_owned())
            .build(k)
            .key(key)
            .focusable(false)
    };
    let actions = row()
        .key("actions")
        .gap(2.0)
        .group_opacity(0.0, 1.0)
        .child(
            action(&icons::SQUARE_ARROW_TOP_RIGHT, "Open in new window", "window")
                .on_press(move |s: &mut Bezel| s.open(&window, Opening::Window, false)),
        )
        .child(action(&icons::PENCIL_LINE, "Edit", "edit").on_press(move |s: &mut Bezel| s.edit_connection(&edit)));
    let mut sidebar_row = SidebarRow::new(name)
        .icon(&icons::MONITOR)
        .selected(dot.is_some())
        .trailing(actions);
    if let Some(dot) = dot {
        let color = match dot {
            Dot::Connected => k.c.success.icon,
            Dot::Connecting => k.c.warning.icon,
        };
        sidebar_row = sidebar_row.status(RowStatus::Other(el().size(7.0, 7.0).radius(3.5).fill(color)));
    }
    let (open, menu, drag) = (id.clone(), id.clone(), id.clone());
    let groups: Vec<(GroupId, String)> = snapshot.groups.iter().map(|g| (g.id.clone(), g.name.clone())).collect();
    let current = snapshot.connection(id).and_then(|c| c.group.clone());
    let targets = targets.clone();
    let built = sidebar_row
        // ⌥-click opens another Session of a Connection that is already open.
        .on_select(move |s: &mut Bezel, tx: &mut Tx| s.open(&open, Opening::Tab, tx.mods().alt))
        .build(k)
        .key(id.to_string())
        .on_secondary_press(move |_: &mut Bezel, press: PointerPress, tx: &mut Tx| {
            let (id, groups) = (menu.clone(), groups.clone());
            tx.popup_menu(
                connection_menu(&groups, current.as_ref()),
                press,
                move |s: &mut Bezel, picked, _| on_connection_menu(s, id, &groups, picked),
            );
        })
        .on_drag(move |s: &mut Bezel, d: &mut Drag, tx: &mut Tx| {
            s.drag(Dragged::Connection(drag.clone()), d, &targets, tx)
        });
    match &s.dragging {
        Some(d) if d.item == Dragged::Connection(id.clone()) => {
            k.elevate(built.offset(d.dx, d.dy).raise().fill(k.c.pressed), &k.elevation.raised)
        }
        _ => built,
    }
}

/// A Connection's context menu. Items for Groups are named by their place in `groups`.
fn connection_menu(groups: &[(GroupId, String)], current: Option<&GroupId>) -> PopupMenu {
    let move_to = groups.iter().enumerate().fold(
        PopupMenu::new().item(PopupItem::new("ungroup", "Ungrouped").checked(current.is_none())),
        |menu, (i, (group, name))| {
            menu.item(PopupItem::new(format!("move-{i}"), name.clone()).checked(current == Some(group)))
        },
    );
    PopupMenu::new()
        .item(PopupItem::new("open", "Open"))
        .item(PopupItem::new("window", "Open in new window"))
        .item(PopupItem::new("edit", "Edit…"))
        .item(PopupItem::new("duplicate", "Duplicate"))
        .submenu("Move to group", move_to)
        .separator()
        .item(PopupItem::new("delete", "Delete…"))
}

fn on_connection_menu(s: &mut Bezel, id: ConnectionId, groups: &[(GroupId, String)], picked: Option<&str>) {
    match picked {
        Some("open") => s.open(&id, Opening::Tab, false),
        Some("window") => s.open(&id, Opening::Window, false),
        Some("edit") => s.edit_connection(&id),
        Some("duplicate") => s.duplicate_connection(&id),
        Some("delete") => s.ask_delete(Delete::Connection(id)),
        Some("ungroup") => s.move_connection(id, None),
        Some(other) => {
            let group = other
                .strip_prefix("move-")
                .and_then(|i| groups.get(i.parse::<usize>().ok()?));
            if let Some((group, _)) = group {
                s.move_connection(id, Some(group.clone()));
            }
        }
        None => {}
    }
}

fn group_menu(id: GroupId, index: usize, count: usize, press: PointerPress, tx: &mut Tx) {
    let menu = PopupMenu::new()
        .item(PopupItem::new("new", "New connection…"))
        .item(PopupItem::new("rename", "Rename…"))
        .item(PopupItem::new("up", "Move up").enabled(index > 0))
        .item(PopupItem::new("down", "Move down").enabled(index + 1 < count))
        .separator()
        .item(PopupItem::new("delete", "Delete…"));
    tx.popup_menu(menu, press, move |s: &mut Bezel, picked, _| match picked {
        Some("new") => s.new_connection(Some(id)),
        Some("rename") => s.rename_group(&id),
        Some("up") => s.step_group(&id, -1),
        Some("down") => s.step_group(&id, 1),
        Some("delete") => s.ask_delete(Delete::Group(id)),
        _ => {}
    });
}

fn foot(s: &Bezel, k: &Kit) -> El {
    let ready = s.library.ready().is_some();
    // Text alone, so Settings and + fit beside them.
    let link = |label: &'static str, key: &'static str, act: fn(&mut Bezel)| {
        Button::new(label)
            .variant(ButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .disabled(!ready)
            .on_press(act)
            .build(k)
            .key(key)
    };
    let settings = IconButton::new(&icons::SETTINGS_GEAR)
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .label("Settings")
        .shortcut("⌘,")
        .disabled(!ready)
        .on_press(Bezel::open_settings)
        .build(k)
        .key("sidebar-settings");
    let add = IconButton::new(&icons::PLUS)
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .label("Add")
        .disabled(!ready)
        .on_press_tx(|_: &mut Bezel, tx: &mut Tx| {
            let menu = PopupMenu::new()
                .item(PopupItem::new("connection", "New connection…"))
                .item(PopupItem::new("group", "New group…"));
            tx.popup_menu(menu, "sidebar-add", |s: &mut Bezel, picked, _| match picked {
                Some("connection") => s.new_connection(None),
                Some("group") => s.new_group(),
                _ => {}
            });
        })
        .build(k)
        .key("sidebar-add");
    row()
        .w_full()
        .gap(2.0)
        .center_y()
        .child(link("Credentials", "sidebar-credentials", |s| s.open_credentials(None)))
        .child(link("Certificates", "sidebar-certificates", Bezel::open_certificates))
        .child(el().grow(1.0))
        .child(settings)
        .child(add)
}
