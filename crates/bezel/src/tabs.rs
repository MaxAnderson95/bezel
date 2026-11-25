//! The Main window's titlebar band with the tab row and the current Session's actions, and a
//! Session window's thin title bar.

use rune::{Cx, El, PointerPress, PopupItem, PopupMenu, Tx, el, row, text};
use rune_kit::shell::{TabItem, TabOrientation, TabStrip, TearOutSpec};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit};
use rune_theme_opencode::icons;

use crate::app::{Bezel, Dot, Screen, Session, SessionId};
use crate::sidebar;
use crate::view::status_line;
use crate::windows::TITLE_BAR;

const TITLEBAR: f32 = 40.0;

fn dot_color(k: &Kit, session: &Session) -> rune::Color {
    match (session.dot(), &session.screen) {
        (Some(Dot::Connected), _) => k.c.success.icon,
        (Some(Dot::Connecting), _) => k.c.warning.icon,
        (None, Screen::Ended(_)) => k.c.danger.icon,
        (None, _) => k.c.icon_faint,
    }
}

/// A Tab's status dot, in the place the strip gives an avatar.
fn dot(k: &Kit, session: &Session) -> El {
    row()
        .size(16.0, 16.0)
        .center()
        .child(el().size(8.0, 8.0).radius(4.0).fill(dot_color(k, session)))
        .a11y_hidden()
}

fn icon_button(glyph: &'static rune::Icon, label: &str) -> IconButton {
    IconButton::new(glyph)
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .label(label.to_owned())
}

pub fn titlebar(s: &Bezel, k: &Kit, cx: &mut Cx<Bezel>, layer: &mut Vec<El>) -> El {
    let toggle = icon_button(&icons::SIDEBAR, "")
        .label(if s.sidebar { "Hide sidebar" } else { "Show sidebar" })
        .shortcut("⌃⌘S")
        .on_press(Bezel::toggle_sidebar)
        .build(k)
        .key("sidebar-toggle");
    let mut bar = row()
        .key("titlebar")
        .w_full()
        .h(TITLEBAR)
        .pl(cx.titlebar_inset() + 8.0)
        .pr(12.0)
        .gap(12.0)
        .center_y()
        .window_drag()
        .child(toggle);
    if s.sidebar {
        // The tab row starts over the panel, clear of the sidebar.
        bar = bar.child(el().w((sidebar::WIDTH - cx.titlebar_inset() - 8.0 - 24.0 - 12.0 * 2.0).max(0.0)));
    }
    let active = s.active.map(|id| id.to_string());
    let items: Vec<TabItem<'_, Bezel>> = s
        .tabs()
        .map(|session| {
            let id = session.id;
            TabItem::new(id.to_string(), session.to.name.as_str())
                .avatar(dot(k, session))
                .decorate(move |tab, _| {
                    tab.on_secondary_press(move |_: &mut Bezel, press: PointerPress, tx: &mut Tx| {
                        tab_menu(id, press, tx)
                    })
                })
        })
        .collect();
    let plus = IconButton::new(&icons::PLUS)
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Large)
        .label("Connect")
        .shortcut("⌘K")
        .on_press_tx(|s: &mut Bezel, tx: &mut Tx| s.open_palette(tx))
        .build(k)
        .key("new-tab");
    let strip = TabStrip::new("tabs", TabOrientation::Horizontal)
        .label("Sessions")
        .tabs(items)
        .active(active.as_deref())
        .viewport_width(cx.layout_size("tabs").map(|size| size[0]))
        .drag_state(&s.tabs_drag, |s: &mut Bezel| &mut s.tabs_drag)
        .tear_out(Some(TearOutSpec {
            lone: true,
            ..TearOutSpec::default()
        }))
        .close_icon(&icons::XMARK_SMALL)
        .trailing_action(plus)
        .on_select(|s: &mut Bezel, id: &str, _: &mut Tx| {
            if let Ok(id) = id.parse() {
                s.reveal(id);
            }
        })
        .on_close(|s: &mut Bezel, id: &str, _: &mut Tx| {
            if let Ok(id) = id.parse() {
                s.close(id);
            }
        })
        .on_reorder(|s: &mut Bezel, r, _: &mut Tx| s.reorder_tab(r.from, r.to))
        .on_tear_out(|s: &mut Bezel, t, tx: &mut Tx| {
            let Ok(id) = t.id.parse() else { return };
            // The new window's title bar lands under the pointer.
            let scale = f64::from(tx.scale());
            let at = t
                .screen_point
                .map(|(x, y)| (x - (120.0 * scale) as i32, y - (16.0 * scale) as i32));
            s.move_to_window(id, at);
        })
        .build_layered(k, layer);
    bar = bar.child(
        row()
            .key("tab-row")
            .grow(1.0)
            .min_w(0.0)
            .h_full()
            .center_y()
            .on_layout_tx(|s: &mut Bezel, rect, tx: &mut Tx| {
                s.tab_row = Some(rect);
                tx.live_only();
            })
            .child(strip),
    );
    if let Some(session) = s.active.and_then(|id| s.session(id)) {
        bar = bar.child(main_actions(session, k));
    }
    bar
}

fn tab_menu(id: SessionId, press: PointerPress, tx: &mut Tx) {
    let menu = PopupMenu::new()
        .item(PopupItem::new("window", "Move to new window"))
        .separator()
        .item(PopupItem::new("close", "Close tab"));
    tx.popup_menu(menu, press, move |s: &mut Bezel, picked, _| match picked {
        Some("window") => s.move_to_window(id, None),
        Some("close") => s.close(id),
        _ => {}
    });
}

/// The current Tab's actions at the right of the tab row.
fn main_actions(session: &Session, k: &Kit) -> El {
    let id = session.id;
    let live = session.live().is_some();
    row()
        .key("tab-actions")
        .gap(2.0)
        .center_y()
        .child(
            icon_button(&icons::KEYBOARD, "Send Ctrl+Alt+Del")
                .disabled(!live)
                .on_press(move |s: &mut Bezel| s.send_ctrl_alt_del(id))
                .build(k)
                .key("ctrl-alt-del"),
        )
        .child(
            icon_button(&icons::SQUARE_ARROW_TOP_RIGHT, "Move to new window")
                .on_press(move |s: &mut Bezel| s.move_to_window(id, None))
                .build(k)
                .key("move-to-window"),
        )
        .child(
            icon_button(&icons::OUTLINE_MAXIMIZE, "Full screen")
                .shortcut("⌃⌘F")
                .on_press_tx(|_: &mut Bezel, tx: &mut Tx| tx.toggle_full_screen())
                .build(k)
                .key("full-screen"),
        )
        .child(more(session, k))
}

/// The … menu: Disconnect, Save as connection for a New host, and Copy details.
fn more(session: &Session, k: &Kit) -> El {
    let id = session.id;
    let new_host = session.to.connection.is_none();
    let details = details(session);
    icon_button(&icons::OUTLINE_DOTS, "More")
        .on_press_tx(move |_: &mut Bezel, tx: &mut Tx| {
            let mut menu = PopupMenu::new().item(PopupItem::new("disconnect", "Disconnect"));
            if new_host {
                menu = menu.item(PopupItem::new("save", "Save as connection…"));
            }
            menu = menu.separator().item(PopupItem::new("copy", "Copy details"));
            let details = details.clone();
            tx.popup_menu(
                menu,
                format!("more-{id}"),
                move |s: &mut Bezel, picked, tx| match picked {
                    Some("disconnect") => s.close(id),
                    Some("save") => s.save_as_connection(id),
                    Some("copy") => tx.copy(details.clone()),
                    _ => {}
                },
            );
        })
        .build(k)
        .key(format!("more-{id}"))
}

/// What Copy details copies for a Session: where it goes, as whom, how it stands, and the
/// versions.
pub fn details(session: &Session) -> String {
    let address = &session.to.address;
    let mut lines = vec![
        session.to.name.clone(),
        format!("Host: {}:{}", address.host, address.port),
    ];
    match &session.screen {
        Screen::Live(live) => {
            lines.push(format!("Username: {}", live.username));
            lines.push(match live.state {
                crate::app::Phase::Connected { width, height } => format!("Connected at {width} × {height}"),
                crate::app::Phase::Connecting => "Connecting".into(),
                crate::app::Phase::Reconnecting => "Reconnecting".into(),
            });
        }
        Screen::Ended(ended) => lines.push(ended.details.clone()),
        Screen::SignIn(_) | Screen::Unlocking(_) | Screen::Retry(_) => {}
    }
    lines.push(format!(
        "Bezel {}, IronRDP {}",
        env!("CARGO_PKG_VERSION"),
        bezel_rdp::IRONRDP_REVISION
    ));
    lines.join("\n")
}

/// A Session window's title bar: the Connection's name, the status line on hover, and Send
/// Ctrl+Alt+Del, Move to main window and the … menu. Dragging it moves the window, and onto
/// the Main window's tab row makes it a Tab again.
pub fn session_title_bar(s: &Bezel, session: &Session, k: &Kit, inset: f32) -> El {
    let id = session.id;
    let mut bar = row()
        .key("session-titlebar")
        .w_full()
        .h(TITLE_BAR)
        .pl(inset + 8.0)
        .pr(8.0)
        .gap(12.0)
        .center_y()
        .hover_group()
        .on_drag(move |s: &mut Bezel, d, tx: &mut Tx| s.drag_window(id, d, tx))
        .child(
            el().size(8.0, 8.0)
                .radius(4.0)
                .fill(dot_color(k, session))
                .a11y_hidden(),
        )
        .child(
            text(session.to.name.clone())
                .font(k.ty.ui_medium)
                .color(k.c.text_base)
                .shrink()
                .truncate(),
        );
    if let Some(live) = session.live() {
        bar = bar.child(
            status_line(s, session, live, k)
                .key("status-line")
                .shrink()
                .min_w(0.0)
                .group_opacity(0.0, 1.0),
        );
    }
    bar.child(el().grow(1.0))
        .child(
            icon_button(&icons::KEYBOARD, "Send Ctrl+Alt+Del")
                .disabled(session.live().is_none())
                .on_press(move |s: &mut Bezel| s.send_ctrl_alt_del(id))
                .build(k)
                .key("ctrl-alt-del"),
        )
        .child(
            icon_button(&icons::ARROW_UNDO_DOWN, "Move to main window")
                .on_press(move |s: &mut Bezel| s.move_to_main(id))
                .build(k)
                .key("move-to-main"),
        )
        .child(more(session, k))
}
