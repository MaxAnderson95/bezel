//! Bezel's windows as drawn. The Main window has a deep titlebar band with the tab row, the
//! sidebar, and the raised panel holding the current Tab's Session or the connect box. A
//! Session window has a thin title bar over its Session. Every colour, type style and radius
//! comes from the kit.

use bezel_core::library::{Credential, CredentialId};
use bezel_core::trust::{Certificate, Fingerprint, Question};
use bezel_rdp::{Ended, Input, MouseButton};
use rune::{Cx, El, Phase as DragPhase, PointerButton, PopupMenu, Tx, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, Checkbox, ControlSize, Dialog, Field, Kit, Select, SelectAppearance, Spinner, TextInput,
};
use rune_theme_opencode::icons;

use crate::app::{Bezel, Library, Live, Phase, Prompt, Retry, Screen, Session, SessionId, SignIn, Trust};
use crate::windows::Window;
use crate::{palette, pointer, sheets, sidebar, tabs};

/// The bezel: the gap between the panel's edge and the remote desktop.
const INSET: f32 = 4.0;

pub fn view(s: &Bezel, cx: &mut Cx<Bezel>) -> El {
    match s.here.window {
        Window::Main => main_window(s, cx),
        Window::Session(id) => session_window(s, id, cx),
    }
}

fn main_window(s: &Bezel, cx: &mut Cx<Bezel>) -> El {
    let k = rune_theme_opencode::kit(s.dark(cx.system_dark()));
    let scale = cx.env().scale;
    let full = cx.is_full_screen();
    let mut layer = Vec::new();
    sheets::sheet(s, k, &mut layer);
    let current = s.active.and_then(|id| s.session(id));
    let mut content = match current {
        Some(session) => stage(s, session, k, scale, &mut layer),
        None => empty(s, k),
    };
    if let (Some(message), Some(_)) = (&s.notice, current) {
        content = content.child(
            col().absolute(16.0, 16.0).w(420.0).child(
                notice(k, message, "", false).child(
                    Button::new("Dismiss")
                        .control_size(ControlSize::Small)
                        .on_press(Bezel::dismiss_notice)
                        .build(k),
                ),
            ),
        );
    }
    sheets::confirm(s, k, &mut layer);
    palette::floating(s, k, &mut layer);
    // Last, so the question is the top modal over any palette or sheet.
    quit_dialog(s, k, &mut layer);
    let area = col()
        .key("area")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .on_drawn_layout_tx(|s: &mut Bezel, rect, tx: &mut Tx| s.layout_area(rect, tx))
        .child(content);
    let mut inside = col().w_full().grow(1.0).min_h(0.0);
    if let Some((session, live)) = current.and_then(|c| Some((c, c.live()?))) {
        inside = inside.child(status_line(s, session, live, k).key("status-line").px(8.0).h(24.0));
    }
    let panel = panel(k, inside.child(area));
    let titlebar = tabs::titlebar(s, k, cx, &mut layer);
    let window = if full {
        let mut window = col().w_full().h_full().fill(k.c.bg_deep).child(panel);
        window = if s.here.reveal {
            window.child(
                col()
                    .key("revealed")
                    .absolute(0.0, 0.0)
                    .w_full()
                    .fill(k.c.bg_deep)
                    .on_hover_change(|s: &mut Bezel, inside, _: &mut Tx| s.here.reveal = inside)
                    .child(titlebar),
            )
        } else {
            window.child(
                el().key("reveal-edge")
                    .absolute(0.0, 0.0)
                    .w_full()
                    .h(4.0)
                    .on_hover_change(|s: &mut Bezel, inside, _: &mut Tx| s.here.reveal |= inside),
            )
        };
        window
    } else {
        let mut body = row().w_full().grow(1.0).min_h(0.0);
        if s.sidebar {
            body = body.child(sidebar::sidebar(s, k));
        }
        body = body.child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .h_full()
                .pr(8.0)
                .pb(8.0)
                .pl(if s.sidebar { 0.0 } else { 8.0 })
                .child(panel),
        );
        col().w_full().h_full().fill(k.c.bg_deep).child(titlebar).child(body)
    };
    rune_kit::root(window, layer)
}

/// The raised, rounded panel the remote desktop sits in.
fn panel(k: &Kit, inside: El) -> El {
    k.elevate(
        col()
            .key("panel")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .padding(INSET)
            .radius(k.radius.lg)
            .fill(k.c.bg_base),
        &k.elevation.raised,
    )
    .clip_rounded(k.radius.lg)
    .child(inside)
}

fn session_window(s: &Bezel, id: SessionId, cx: &mut Cx<Bezel>) -> El {
    let k = rune_theme_opencode::kit(s.dark(cx.system_dark()));
    let scale = cx.env().scale;
    let mut layer = Vec::new();
    let Some(session) = s.session(id) else {
        return rune_kit::root(col().w_full().h_full().fill(k.c.bg_deep), layer);
    };
    let content = stage(s, session, k, scale, &mut layer);
    palette::floating(s, k, &mut layer);
    let area = col()
        .key("area")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .on_drawn_layout_tx(|s: &mut Bezel, rect, tx: &mut Tx| s.layout_area(rect, tx))
        .child(content);
    let window = col()
        .w_full()
        .h_full()
        .fill(k.c.bg_deep)
        .child(tabs::session_title_bar(s, session, k, cx.titlebar_inset()))
        .child(
            col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .px(6.0)
                .pb(6.0)
                .child(panel(k, area)),
        );
    rune_kit::root(window, layer)
}

/// What a Tab or Session window shows of its Session, with its dialogs in `layer`.
fn stage(s: &Bezel, session: &Session, k: &Kit, scale: f32, layer: &mut Vec<El>) -> El {
    let id = session.id;
    let shown = match &session.screen {
        Screen::SignIn(sign_in) => {
            sign_in_dialog(s, session, sign_in, k, layer);
            el()
        }
        Screen::Unlocking(_) => connecting(k, &session.to.name),
        Screen::Retry(retry) => retry_card(s, session, retry, k),
        Screen::Live(live) => {
            if let Some(prompt) = &live.certificate {
                certificate_dialog(id, prompt, k, &s.library, layer);
            }
            desktop(s, session, live, k, scale)
        }
        Screen::Ended(ended) => col()
            .w_full()
            .h_full()
            .center()
            .padding(24.0)
            .child(col().w_full().max_w(440.0).child(ended_card(Some(id), ended, k))),
    };
    let mut stage = col().w_full().h_full().child(shown);
    if let Some(message) = &session.notice {
        stage = stage.child(
            col().absolute(16.0, 16.0).w(420.0).child(
                notice(k, message, "", false).child(
                    Button::new("Dismiss")
                        .control_size(ControlSize::Small)
                        .on_press(move |s: &mut Bezel| s.dismiss_session_notice(id))
                        .build(k),
                ),
            ),
        );
    }
    stage
}

/// The status line: host, username, the live remote size, and "Trusted" when the certificate
/// was verified.
pub fn status_line(s: &Bezel, session: &Session, live: &Live, k: &Kit) -> El {
    let (dot, state) = match live.state {
        Phase::Connected { width, height } => {
            let retina = s.area_of(s.here.window).is_some_and(|a| a.scale > 1.0);
            (
                k.c.success.icon,
                format!("{width} × {height}{}", if retina { " Retina" } else { "" }),
            )
        }
        Phase::Connecting => (k.c.warning.icon, "Connecting…".into()),
        Phase::Reconnecting => (k.c.warning.icon, "Reconnecting…".into()),
    };
    let mut line = row()
        .gap(12.0)
        .center_y()
        .child(el().size(8.0, 8.0).radius(4.0).fill(dot).a11y_hidden())
        .child(
            text(format!("{}:{}", session.to.address.host, session.to.address.port))
                .font(k.ty.mono)
                .color(k.c.text_base),
        )
        .child(text(live.username.clone()).font(k.ty.ui).color(k.c.text_muted))
        .child(text(state).font(k.ty.ui).color(k.c.text_muted));
    if matches!(live.state, Phase::Connected { .. }) && live.trust == Trust::Verified {
        line = line.child(
            row()
                .gap(4.0)
                .center_y()
                .child(icon(&icons::SHIELD, 16.0).color(k.c.icon_muted))
                .child(text("Trusted").font(k.ty.ui).color(k.c.text_muted)),
        );
    }
    line
}

/// The Main window's panel with no Tab: the palette's field inline, and on first run a way
/// to add a Connection.
fn empty(s: &Bezel, k: &Kit) -> El {
    let mut column = col().w_full().max_w(440.0).gap(12.0).child(palette::inline(s, k));
    if let Library::Failed(error) = &s.library {
        column = column.child(notice(
            k,
            error,
            "Bezel can't show or save connections, credentials or certificates until the file is fixed.",
            true,
        ));
    }
    if s.library.ready().is_some_and(|l| l.connections.is_empty()) {
        column = column.child(
            row().w_full().child(
                Button::new("Add connection")
                    .icon(&icons::PLUS)
                    .on_press(|s: &mut Bezel| s.new_connection(None))
                    .build(k)
                    .key("add-connection"),
            ),
        );
    }
    if let Some(error) = &s.notice {
        column = column.child(
            notice(k, error, "", false).child(
                Button::new("Dismiss")
                    .control_size(ControlSize::Small)
                    .on_press(Bezel::dismiss_notice)
                    .build(k),
            ),
        );
    }
    col().w_full().h_full().center().padding(24.0).child(column)
}

fn connecting(k: &Kit, name: &str) -> El {
    col()
        .w_full()
        .h_full()
        .center()
        .gap(12.0)
        .child(Spinner::new().build(k))
        .child(
            text(format!("Connecting to {name}…"))
                .font(k.ty.ui)
                .color(k.c.text_muted),
        )
}

/// ⌘Q with Sessions open.
fn quit_dialog(s: &Bezel, k: &Kit, layer: &mut Vec<El>) {
    if !s.quitting {
        return;
    }
    let open = s.sessions.iter().filter(|x| x.is_open()).count();
    let title = match open {
        1 => "Disconnect 1 Session and quit?".to_owned(),
        n => format!("Disconnect {n} Sessions and quit?"),
    };
    let body = col().w_full().px(16.0).pb(16.0).child(buttons(
        Button::new("Cancel")
            .on_press(Bezel::cancel_quit)
            .build(k)
            .key("quit-cancel"),
        Button::new("Disconnect and quit")
            .variant(ButtonVariant::Contrast)
            .on_press_tx(|s: &mut Bezel, tx: &mut Tx| s.quit(tx))
            .build(k)
            .key("quit-confirm")
            .default_button(),
    ));
    Dialog::new("quit", title)
        .description("Windows keeps each user signed in on the server.")
        .open(true)
        .fit()
        .width(400.0)
        .on_close(Bezel::cancel_quit)
        .child(body)
        .build(k, layer);
}

/// The wrong-password prompt in place of the Session: a new password for the Credential, saved
/// for every Connection that uses it.
fn retry_card(s: &Bezel, session: &Session, retry: &Retry, k: &Kit) -> El {
    let id = session.id;
    let credential = &retry.credential;
    let name = &credential.name;
    let users = s.library.ready().map_or(0, |l| l.used_by(&credential.id).len());
    let title = if retry.rejected {
        format!("Sign-in failed for {name}")
    } else {
        format!("No password saved for {name}")
    };
    let mut detail = if retry.rejected {
        format!("{} didn't accept the password. ", session.to.address.host)
    } else {
        String::new()
    };
    detail.push_str(&match users {
        0 | 1 => format!(
            "Enter the password for {name}; Bezel saves it and signs in to {} again.",
            session.to.name
        ),
        n => format!(
            "Enter the password for {name}; Bezel saves it for the {n} connections that use it and signs in to {} again.",
            session.to.name
        ),
    });
    let label = format!("Password for {}", credential.username);
    let password = TextInput::new(&retry.password)
        .label(label.clone())
        .width(None)
        .secure()
        .invalid(retry.error.is_some())
        .on_edit(move |s: &mut Bezel| {
            s.field(id, |screen| match screen {
                Screen::Retry(retry) => Some(&mut retry.password),
                _ => None,
            })
        })
        .on_submit(move |s: &mut Bezel, _: &mut Tx| s.retry(id))
        .build(k)
        .key("retry-password");
    let mut card = col()
        .key("retry")
        .w_full()
        .max_w(440.0)
        .gap(16.0)
        .padding(16.0)
        .radius(k.radius.md)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_base)
        .child(
            col()
                .w_full()
                .gap(4.0)
                .child(text(title).font(k.ty.ui_medium).color(k.c.text_base))
                .child(text(detail).font(k.ty.ui).color(k.c.text_muted).wrap().w_full()),
        )
        .child(Field::new().label(label).control(password).build(k));
    if let Some(error) = &retry.error {
        card = card.child(text(error.clone()).font(k.ty.ui).color(k.c.danger.fg).wrap().w_full());
    }
    let saving = retry.saving();
    card = card.child(buttons(
        Button::new("Cancel")
            .on_press(move |s: &mut Bezel| s.cancel_sign_in(id))
            .build(k),
        Button::new(if saving { "Saving…" } else { "Save and retry" })
            .variant(ButtonVariant::Contrast)
            .disabled(saving)
            .on_press(move |s: &mut Bezel| s.retry(id))
            .build(k)
            .key("retry-save")
            .default_button(),
    ));
    col().w_full().h_full().center().padding(24.0).child(card)
}

/// Why the Session ended, with Copy details, and Close for a Session's own Tab or window.
pub fn ended_card(id: Option<SessionId>, ended: &Ended, k: &Kit) -> El {
    let details = ended.details.clone();
    let mut actions = row().gap(4.0).center_y().child(
        Button::new("Copy details")
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Small)
            .on_press_tx(move |_: &mut Bezel, tx: &mut Tx| tx.copy(details.clone()))
            .build(k),
    );
    if let Some(id) = id {
        actions = actions.child(
            Button::new("Close")
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut Bezel| s.close(id))
                .build(k)
                .key("ended-close"),
        );
    }
    col()
        .key("ended")
        .w_full()
        .gap(8.0)
        .padding(12.0)
        .radius(k.radius.md)
        .fill(k.c.layer_01)
        .border(1.0, k.c.border_base)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .center_y()
                .child(
                    text(ended.message.clone())
                        .font(k.ty.ui)
                        .color(k.c.text_base)
                        .wrap()
                        .grow(1.0)
                        .shrink(),
                )
                .child(actions),
        )
}

pub fn notice(k: &Kit, message: &str, detail: &str, open_folder: bool) -> El {
    let mut card = col()
        .w_full()
        .gap(8.0)
        .padding(12.0)
        .radius(k.radius.md)
        .fill(k.c.warning.bg)
        .border(1.0, k.c.warning.border)
        .child(
            text(message.to_owned())
                .font(k.ty.ui)
                .color(k.c.warning.fg)
                .wrap()
                .w_full(),
        );
    if !detail.is_empty() {
        card = card.child(
            text(detail.to_owned())
                .font(k.ty.ui)
                .color(k.c.text_muted)
                .wrap()
                .w_full(),
        );
    }
    if open_folder {
        card = card.child(
            Button::new("Open data folder")
                .control_size(ControlSize::Small)
                .on_press_tx(|_: &mut Bezel, tx: &mut Tx| {
                    if let Some(folder) = crate::platform::library_file().parent() {
                        let _ = tx.open_path(folder);
                    }
                })
                .build(k),
        );
    }
    card
}

fn sign_in_dialog(s: &Bezel, session: &Session, sign_in: &SignIn, k: &Kit, layer: &mut Vec<El>) {
    let id = session.id;
    let waiting = sign_in.waiting();
    let credentials = s.library.ready().map(|l| l.credentials.as_slice()).unwrap_or_default();
    let mut body = col().w_full().gap(16.0).px(16.0).pb(16.0);
    if !credentials.is_empty() {
        let current = sign_in
            .credential
            .as_ref()
            .and_then(|c| credentials.iter().find(|x| &x.id == c))
            .map_or(TYPE_IT.to_owned(), |c| c.name.clone());
        let choices = std::iter::once((None, TYPE_IT.to_owned()))
            .chain(credentials.iter().map(|c| (Some(c.id.clone()), c.name.clone())))
            .collect();
        body = body.child(
            Field::new()
                .label("Credential")
                .control(picker(
                    k,
                    "sign-in-credential",
                    "Credential",
                    &current,
                    choices,
                    &sign_in.credential,
                    move |s: &mut Bezel, picked| s.sign_in_with(id, picked),
                ))
                .build(k),
        );
    }
    if sign_in.credential.is_none() {
        body = body.child(sign_in_fields(id, sign_in, k));
    }
    if let Some(error) = &sign_in.error {
        body = body.child(text(error.clone()).font(k.ty.ui).color(k.c.danger.fg).wrap().w_full());
    }
    body = body.child(buttons(
        Button::new("Cancel")
            .on_press(move |s: &mut Bezel| s.cancel_sign_in(id))
            .build(k),
        Button::new(if waiting { "Connecting…" } else { "Connect" })
            .variant(ButtonVariant::Contrast)
            .disabled(waiting)
            .on_press(move |s: &mut Bezel| s.sign_in(id))
            .build(k)
            .key("sign-in-connect")
            .default_button(),
    ));
    Dialog::new("sign-in", format!("Sign in to {}", session.to.name))
        .open(true)
        .fit()
        .width(400.0)
        .on_close(move |s: &mut Bezel| s.cancel_sign_in(id))
        .child(body)
        .build(k, layer);
}

/// The sign-in picker's choice for typing a username and password.
const TYPE_IT: &str = "Type a username and password";

fn sign_in_fields(id: SessionId, sign_in: &SignIn, k: &Kit) -> El {
    type Pick = fn(&mut Screen) -> Option<&mut rune::TextEdit>;
    let input = move |edit: &rune::TextEdit, label: &'static str, pick: Pick| {
        TextInput::new(edit)
            .label(label)
            .width(None)
            .on_edit(move |s: &mut Bezel| s.field(id, pick))
            .on_submit(move |s: &mut Bezel, _: &mut Tx| s.sign_in(id))
    };
    let username = Field::new()
        .label("Username")
        .control(
            input(&sign_in.username, "Username", |screen| match screen {
                Screen::SignIn(sign_in) => Some(&mut sign_in.username),
                _ => None,
            })
            .placeholder(r"DOMAIN\user, user@domain or user")
            .build(k)
            .key("sign-in-username"),
        )
        .build(k);
    let password = Field::new()
        .label("Password")
        .control(
            input(&sign_in.password, "Password", |screen| match screen {
                Screen::SignIn(sign_in) => Some(&mut sign_in.password),
                _ => None,
            })
            .secure()
            .invalid(sign_in.error.is_some())
            .build(k)
            .key("sign-in-password"),
        )
        .build(k);
    let mut fields = col().w_full().gap(16.0).child(username).child(password).child(
        Checkbox::new(sign_in.save)
            .label("Save as credential")
            .on_toggle(move |s: &mut Bezel| s.toggle_save_credential(id))
            .build(k)
            .key("sign-in-save"),
    );
    if sign_in.save {
        fields = fields.child(
            Field::new()
                .label("Credential name")
                .control(
                    input(&sign_in.save_name, "Credential name", |screen| match screen {
                        Screen::SignIn(sign_in) => Some(&mut sign_in.save_name),
                        _ => None,
                    })
                    .placeholder("Domain admin")
                    .build(k)
                    .key("sign-in-save-name"),
                )
                .build(k),
        );
    }
    fields
}

/// A field showing the chosen Credential that opens a native menu of the saved ones, with
/// `none` (Ask each time, or typing one) first.
pub fn credential_picker(
    k: &Kit,
    key: &'static str,
    credentials: &[Credential],
    current: Option<&CredentialId>,
    none: &'static str,
    label: &str,
    pick: fn(&mut Bezel, Option<CredentialId>),
) -> El {
    let choices = std::iter::once((None, none.to_owned()))
        .chain(credentials.iter().map(|c| (Some(c.id.clone()), c.name.clone())))
        .collect();
    picker(k, key, "Credential", label, choices, &current.cloned(), pick)
}

/// The kit's select field as the face of a native pop-up menu of `(value, title)` choices.
/// `name` says what the choice is for; `label` shows the current one.
pub fn picker<T: Clone + PartialEq + 'static>(
    k: &Kit,
    key: &'static str,
    name: &str,
    label: &str,
    choices: Vec<(T, String)>,
    current: &T,
    pick: impl Fn(&mut Bezel, T) + Clone + 'static,
) -> El {
    let menu = picker_menu(&choices, current);
    Select::new(key, &[label])
        .label(name)
        .current(Some(label))
        .appearance(SelectAppearance::Field)
        .fill()
        .on_toggle_tx(move |_: &mut Bezel, tx: &mut Tx| {
            let (choices, pick) = (choices.clone(), pick.clone());
            tx.popup_menu(
                menu.clone(),
                format!("{key}-trigger"),
                move |s: &mut Bezel, picked, _| {
                    let choice = picked.and_then(|id| choices.get(id.parse::<usize>().ok()?));
                    if let Some((value, _)) = choice {
                        pick(s, value.clone());
                    }
                },
            );
        })
        .build(k, &mut Vec::new())
}

/// Items are named by their place in `choices`: Rune refuses a menu with an empty or repeated
/// item ID, and the values themselves may be empty (Ask each time, no Group).
fn picker_menu<T: PartialEq>(choices: &[(T, String)], current: &T) -> PopupMenu {
    let selected = choices.iter().position(|(value, _)| value == current).unwrap_or(0);
    let items = choices
        .iter()
        .enumerate()
        .map(|(i, (_, title))| (i.to_string(), title.clone()));
    PopupMenu::picker(items, &selected.to_string())
}

pub fn buttons(secondary: El, primary: El) -> El {
    row()
        .w_full()
        .gap(8.0)
        .pt(4.0)
        .child(el().grow(1.0))
        .child(secondary)
        .child(primary)
}

fn certificate_dialog(id: SessionId, prompt: &Prompt, k: &Kit, library: &Library, layer: &mut Vec<El>) {
    let host = &prompt.request.host;
    // Without its saved certificates Bezel can't tell a new certificate from a changed one,
    // and can't save one either.
    let unreadable = matches!(library, Library::Failed(_));
    let trust = move |s: &mut Bezel| s.trust(id);
    let refuse = move |s: &mut Bezel| s.refuse_certificate(id);
    let dialog = match &prompt.request.question {
        Question::FirstUse(certificate) => {
            let body = col()
                .w_full()
                .gap(16.0)
                .px(16.0)
                .pb(16.0)
                .child(facts(k, certificate))
                .child(
                    Checkbox::new(prompt.remember)
                        .label("Remember this certificate")
                        .description(if unreadable {
                            "Bezel can't read its saved certificates, so it can't tell whether this one changed or remember it"
                        } else {
                            ""
                        })
                        .disabled(unreadable)
                        .on_toggle(move |s: &mut Bezel| s.toggle_remember(id))
                        .build(k)
                        .key("certificate-remember"),
                )
                .child(buttons(
                    Button::new("Cancel").on_press(refuse).build(k),
                    Button::new("Trust and connect")
                        .variant(ButtonVariant::Contrast)
                        .on_press(trust)
                        .build(k)
                        .key("certificate-trust")
                        .default_button(),
                ));
            Dialog::new("certificate", format!("Bezel can't verify {host}"))
                .description("The certificate isn't from an authority this Mac trusts. Check its fingerprint with the server's administrator before trusting it.")
                .child(body)
        }
        Question::Changed { presented, .. } if prompt.confirming => {
            let body = col().w_full().gap(16.0).px(16.0).pb(16.0).child(buttons(
                Button::new("Cancel")
                    .on_press(move |s: &mut Bezel| s.confirm_change(id, false))
                    .build(k)
                    .default_button(),
                Button::new("Trust new certificate")
                    .variant(ButtonVariant::Danger)
                    .on_press(trust)
                    .build(k),
            ));
            Dialog::new("certificate", format!("Trust the new certificate for {host}?"))
                .description(format!(
                    "Bezel will forget the certificate it trusted and remember the one with fingerprint {}.",
                    grouped(presented.sha256)
                ))
                .child(body)
        }
        Question::Changed { trusted, presented } => {
            let old = grouped(trusted.sha256);
            let changed = col()
                .w_full()
                .gap(8.0)
                .child(fact(k, "Trusted", date(trusted.remembered)))
                .child(fact_el(
                    k,
                    "Old SHA-256",
                    fingerprint_text(k, &old).strike(0, old.len(), None),
                ))
                .child(fact_el(
                    k,
                    "New SHA-256",
                    fingerprint_text(k, &grouped(presented.sha256)),
                ))
                .child(fact(k, "Issued by", issuer(presented)))
                .child(fact(k, "Issued", date(presented.not_before)));
            let body = col().w_full().gap(16.0).px(16.0).pb(16.0).child(changed).child(buttons(
                Button::new("Trust new certificate…")
                    .variant(ButtonVariant::Danger)
                    .on_press(move |s: &mut Bezel| s.confirm_change(id, true))
                    .build(k),
                Button::new("Don't connect").on_press(refuse).build(k).default_button(),
            ));
            Dialog::new("certificate", format!("{host} has a different certificate"))
                .description("Someone may be intercepting the connection, or the server's certificate was replaced.")
                .child(body)
        }
    };
    dialog.open(true).fit().width(480.0).on_close(refuse).build(k, layer);
}

fn facts(k: &Kit, certificate: &Certificate) -> El {
    col()
        .w_full()
        .gap(8.0)
        .child(fact(k, "Issued to", certificate.subject.clone()))
        .child(fact(k, "Issued by", issuer(certificate)))
        .child(fact(k, "Expires", date(certificate.not_after)))
        .child(fact_el(k, "SHA-256", fingerprint_text(k, &grouped(certificate.sha256))))
}

pub fn fact(k: &Kit, label: &str, value: String) -> El {
    fact_el(k, label, text(value).font(k.ty.ui).color(k.c.text_base).wrap())
}

pub fn fact_el(k: &Kit, label: &str, value: El) -> El {
    row()
        .w_full()
        .gap(12.0)
        .items_start()
        .child(text(label.to_owned()).font(k.ty.ui).color(k.c.text_muted).w(96.0))
        .child(value.grow(1.0).shrink())
}

pub fn fingerprint_text(k: &Kit, fingerprint: &str) -> El {
    text(fingerprint.to_owned())
        .font(k.ty.mono)
        .color(k.c.text_base)
        .wrap()
        .selectable(true)
}

fn issuer(certificate: &Certificate) -> String {
    if certificate.self_signed {
        format!("{} (self-signed)", certificate.issuer)
    } else {
        certificate.issuer.clone()
    }
}

pub fn date(date: time::Date) -> String {
    format!("{} {}, {}", date.month(), date.day(), date.year())
}

/// Groups of four hex digits, as fingerprints are read aloud and compared.
pub fn grouped(fingerprint: Fingerprint) -> String {
    let hex = fingerprint.to_string();
    hex.as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The element key of the Session's remote desktop. Each Session has its own, so switching
/// Tabs mounts the new one, which takes the keyboard.
pub fn desktop_key(id: SessionId) -> String {
    format!("desktop-{id}")
}

fn desktop(s: &Bezel, session: &Session, live: &Live, k: &Kit, scale: f32) -> El {
    let Some(area) = s.area_of(s.here.window) else {
        return el();
    };
    let id = session.id;
    let frame = match live.state {
        Phase::Connected { width, height } => Some((width, height)),
        Phase::Connecting | Phase::Reconnecting => {
            let (width, height) = live.handle.framebuffer().size();
            (width > 0).then_some((width as u16, height as u16))
        }
    };
    let Some(frame) = frame else {
        return connecting(k, &session.to.name);
    };
    let [width, height] = desktop_size([area.width, area.height], frame, scale);
    let key = desktop_key(id);
    let focus = key.clone();
    let refocus = key.clone();
    let mut desktop = el()
        .key(key)
        .label("Remote desktop")
        .size(width, height)
        .pixels(live.handle.framebuffer())
        .focusable(true)
        .cursor(pointer::cursor(&live.pointer, scale))
        .pointer_buttons([
            PointerButton::Secondary,
            PointerButton::Middle,
            PointerButton::Back,
            PointerButton::Forward,
        ])
        .on_mount(move |_: &mut Bezel, tx: &mut Tx| tx.focus(focus.clone(), false))
        .on_pointer_move_local(move |s: &mut Bezel, p, tx: &mut Tx| {
            s.input(
                id,
                Input::PointerMove {
                    x: p.x / width,
                    y: p.y / height,
                },
            );
            tx.live_only();
        })
        .on_drag(move |s: &mut Bezel, d, tx: &mut Tx| {
            let button = match tx.button() {
                PointerButton::Primary => MouseButton::Left,
                PointerButton::Secondary => MouseButton::Right,
                PointerButton::Middle => MouseButton::Middle,
                PointerButton::Back => MouseButton::Back,
                PointerButton::Forward => MouseButton::Forward,
                _ => return,
            };
            s.input(
                id,
                Input::PointerMove {
                    x: d.x / width,
                    y: d.y / height,
                },
            );
            match d.phase {
                DragPhase::Down => {
                    tx.focus(refocus.clone(), false);
                    s.input(id, Input::Button { button, pressed: true });
                }
                DragPhase::Up => s.input(id, Input::Button { button, pressed: false }),
                DragPhase::Move => tx.live_only(),
            }
        })
        .on_wheel(move |s: &mut Bezel, e, tx: &mut Tx| {
            s.input(id, Input::Wheel { dx: e.dx, dy: e.dy });
            tx.live_only();
            true
        })
        .on_physical_input(move |s: &mut Bezel, e, tx: &mut Tx| s.physical(id, e, tx));
    if live.state == Phase::Reconnecting {
        desktop = desktop.opacity(0.4);
    }
    let mut stage = col().w_full().h_full().child(desktop);
    if live.state == Phase::Reconnecting {
        stage = stage.child(
            col()
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .center()
                .child(text("Reconnecting…").font(k.ty.ui_medium).color(k.c.text_base)),
        );
    }
    stage
}

/// The desktop draws one remote pixel per screen pixel when the frame matches the area (to
/// within the even-width rounding), and stretches to fit while a resize is on its way.
fn desktop_size(area: [f32; 2], frame: (u16, u16), scale: f32) -> [f32; 2] {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let frame = [f32::from(frame.0) / scale, f32::from(frame.1) / scale];
    let close = |a: f32, f: f32| (a - f).abs() <= 2.0 / scale;
    if close(area[0], frame[0]) && close(area[1], frame[1]) {
        frame
    } else {
        area
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_matching_frame_draws_one_to_one() {
        assert_eq!(desktop_size([1023.5, 700.0], (2046, 1400), 2.0), [1023.0, 700.0]);
    }

    #[test]
    fn a_frame_of_another_size_stretches_to_the_area() {
        assert_eq!(desktop_size([1200.0, 700.0], (2046, 1400), 2.0), [1200.0, 700.0]);
    }

    #[test]
    fn fingerprints_group_by_four() {
        let grouped = grouped(
            "7F3A91C20D5E88B41A6CE34792FD057BC8219E44A06DF1385B72CE0913B68AE5"
                .parse()
                .unwrap(),
        );
        assert!(grouped.starts_with("7F3A 91C2 0D5E"));
        assert_eq!(grouped.split(' ').count(), 16);
    }
}
