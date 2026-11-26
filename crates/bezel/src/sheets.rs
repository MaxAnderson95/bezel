//! The sheets over the Main window: New connection and Edit, New group and Rename, Credentials,
//! Certificates, and the confirmations for deleting.

use bezel_core::library::{Credential, Snapshot};
use rune::{El, TextEdit, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Dialog, Field, IconButton, IconButtonVariant, Kit, TextInput};
use rune_theme_opencode::icons;

use crate::app::{Bezel, today};
use crate::manage::{ConnectionForm, Credentials, Delete, GroupForm, Selected, Sheet, default_name};
use crate::view::{buttons, credential_picker, date, fact, fact_el, fingerprint_text, grouped, picker};

pub fn sheet(s: &Bezel, k: &Kit, layer: &mut Vec<El>) {
    let Some(snapshot) = s.library.ready() else { return };
    match &s.sheet {
        None => {}
        Some(Sheet::Connection(form)) => connection(form, snapshot, k, layer),
        Some(Sheet::Group(form)) => group(form, k, layer),
        Some(Sheet::Credentials(sheet)) => credentials(sheet, snapshot, k, layer),
        Some(Sheet::Certificates) => certificates(snapshot, k, layer),
    }
}

fn field(k: &Kit, label: &str, control: El) -> El {
    Field::new().label(label).control(control).build(k)
}

fn error(k: &Kit, message: &str) -> El {
    text(message.to_owned())
        .font(k.ty.ui)
        .color(k.c.danger.fg)
        .wrap()
        .w_full()
}

fn connection(form: &ConnectionForm, snapshot: &Snapshot, k: &Kit, layer: &mut Vec<El>) {
    fn edit(s: &mut Bezel, field: fn(&mut ConnectionForm) -> &mut TextEdit) -> &mut TextEdit {
        match &mut s.sheet {
            Some(Sheet::Connection(form)) => field(form),
            _ => &mut s.host,
        }
    }
    let default = match bezel_core::address::Address::parse(&form.host.text) {
        Ok(address) => default_name(&address.host),
        Err(_) => "dc01".into(),
    };
    let name = TextInput::new(&form.name)
        .label("Name")
        .placeholder(default)
        .width(None)
        .on_edit(|s: &mut Bezel| edit(s, |form| &mut form.name))
        .on_submit(|s: &mut Bezel, _: &mut Tx| s.save_connection())
        .build(k)
        .key("connection-name");
    let host = TextInput::new(&form.host)
        .label("Host")
        .placeholder("dc01.corp.lan or 10.0.0.5:3390")
        .width(None)
        .invalid(form.error.is_some())
        .on_edit(|s: &mut Bezel| edit(s, |form| &mut form.host))
        .on_change(|s: &mut Bezel, _: &mut Tx| {
            if let Some(Sheet::Connection(form)) = &mut s.sheet {
                form.error = None;
            }
        })
        .on_submit(|s: &mut Bezel, _: &mut Tx| s.save_connection())
        .build(k)
        .key("connection-host");
    const ASK: &str = "Ask each time";
    let credential = form
        .credential
        .as_ref()
        .and_then(|id| snapshot.credential(id))
        .map_or(ASK.to_owned(), |c| c.name.clone());
    let credential = credential_picker(
        k,
        "connection-credential",
        &snapshot.credentials,
        form.credential.as_ref(),
        ASK,
        &credential,
        Bezel::set_form_credential,
    );
    let group_name = form
        .group
        .as_ref()
        .and_then(|id| snapshot.group(id))
        .map_or("None".to_owned(), |g| g.name.clone());
    let groups = std::iter::once((None, "None".to_owned()))
        .chain(snapshot.groups.iter().map(|g| (Some(g.id.clone()), g.name.clone())))
        .collect();
    let group = picker(
        k,
        "connection-group",
        "Group",
        &group_name,
        groups,
        &form.group,
        Bezel::set_form_group,
    );
    let mut body = col()
        .w_full()
        .gap(16.0)
        .px(16.0)
        .pb(16.0)
        .child(field(k, "Name", name))
        .child(field(k, "Host", host))
        .child(field(k, "Credential", credential))
        .child(field(k, "Group", group));
    if let Some(message) = &form.error {
        body = body.child(error(k, message));
    }
    let saving = form.saving();
    body = body.child(buttons(
        Button::new("Cancel").on_press(Bezel::close_sheet).build(k),
        Button::new(match (saving, form.editing.is_some()) {
            (true, _) => "Saving…",
            (false, true) => "Save",
            (false, false) => "Add connection",
        })
        .variant(ButtonVariant::Contrast)
        .disabled(saving)
        .on_press(Bezel::save_connection)
        .build(k)
        .key("connection-save")
        .default_button(),
    ));
    let title = match &form.editing {
        Some(connection) => format!("Edit {}", connection.name),
        None => "New connection".into(),
    };
    Dialog::new("connection", title)
        .open(true)
        .fit()
        .width(440.0)
        .on_close(Bezel::close_sheet)
        .child(body)
        .build(k, layer);
}

fn group(form: &GroupForm, k: &Kit, layer: &mut Vec<El>) {
    let name = TextInput::new(&form.name)
        .label("Name")
        .placeholder("Infrastructure")
        .width(None)
        .on_edit(|s: &mut Bezel| match &mut s.sheet {
            Some(Sheet::Group(form)) => &mut form.name,
            _ => &mut s.host,
        })
        .on_submit(|s: &mut Bezel, _: &mut Tx| s.save_group())
        .build(k)
        .key("group-name");
    let body = col()
        .w_full()
        .gap(16.0)
        .px(16.0)
        .pb(16.0)
        .child(field(k, "Name", name))
        .child(buttons(
            Button::new("Cancel").on_press(Bezel::close_sheet).build(k),
            Button::new(if form.renaming.is_some() { "Rename" } else { "Add group" })
                .variant(ButtonVariant::Contrast)
                .disabled(form.name.text.trim().is_empty())
                .on_press(Bezel::save_group)
                .build(k)
                .key("group-save")
                .default_button(),
        ));
    Dialog::new(
        "group",
        if form.renaming.is_some() {
            "Rename group"
        } else {
            "New group"
        },
    )
    .open(true)
    .fit()
    .width(360.0)
    .on_close(Bezel::close_sheet)
    .child(body)
    .build(k, layer);
}

/// The searchable list of Credentials beside the selected one's name, username, password and
/// the Connections using it.
fn credentials(sheet: &Credentials, snapshot: &Snapshot, k: &Kit, layer: &mut Vec<El>) {
    let search = TextInput::new(&sheet.search)
        .label("Search credentials")
        .placeholder("Search")
        .leading_icon(k.glyph.search)
        .width(None)
        .on_edit(|s: &mut Bezel| match &mut s.sheet {
            Some(Sheet::Credentials(sheet)) => &mut sheet.search,
            _ => &mut s.host,
        })
        .build(k)
        .key("credentials-search");
    let query = sheet.search.text.trim().to_lowercase();
    let mut rows = col().key("credentials-list").role("list").w_full().gap(2.0);
    let mut shown = 0;
    for credential in &snapshot.credentials {
        let matches = query.is_empty()
            || credential.name.to_lowercase().contains(&query)
            || credential.username.to_lowercase().contains(&query);
        if matches {
            shown += 1;
            let selected = matches!(&sheet.selected, Selected::Existing(c) if c.id == credential.id);
            rows = rows.child(credential_row(
                credential,
                snapshot.used_by(&credential.id).len(),
                selected,
                k,
            ));
        }
    }
    if shown == 0 {
        let message = if snapshot.credentials.is_empty() {
            "No credentials yet"
        } else {
            "No matches"
        };
        rows = rows.child(text(message).font(k.ty.ui).color(k.c.text_faint).px(8.0).py(6.0));
    }
    let add = IconButton::new(&icons::PLUS)
        .variant(IconButtonVariant::GhostMuted)
        .control_size(ControlSize::Small)
        .label("New credential")
        .on_press(Bezel::new_credential)
        .build(k)
        .key("credentials-new");
    let list = col()
        .w(240.0)
        .h_full()
        .gap(8.0)
        .pr(12.0)
        .child(
            row()
                .w_full()
                .gap(4.0)
                .center_y()
                .child(search.grow(1.0).shrink())
                .child(add),
        )
        .child(
            col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(Default::default())
                .child(rows),
        );
    let detail = match &sheet.selected {
        Selected::Nothing => col()
            .grow(1.0)
            .h_full()
            .center()
            .child(text("Select a credential").font(k.ty.ui).color(k.c.text_faint)),
        Selected::New(_) => credential_detail(sheet, None, snapshot, k),
        Selected::Existing(selected) => match snapshot.credential(&selected.id) {
            Some(credential) => credential_detail(sheet, Some(credential), snapshot, k),
            None => col()
                .grow(1.0)
                .h_full()
                .center()
                .child(text("This credential was deleted").font(k.ty.ui).color(k.c.text_faint)),
        },
    };
    let body = row()
        .w_full()
        .h(400.0)
        .px(16.0)
        .pb(16.0)
        .child(list)
        .child(el().w(1.0).h_full().fill(k.c.border_weak))
        .child(detail);
    Dialog::new("credentials", "Credentials")
        .open(true)
        .fit()
        .width(720.0)
        .on_close(Bezel::close_sheet)
        .child(body)
        .build(k, layer);
}

fn credential_row(credential: &Credential, users: usize, selected: bool, k: &Kit) -> El {
    let id = credential.id.clone();
    let mut item = row()
        .key(format!("credential-{}", credential.id))
        .role("listitem")
        .label(credential.name.clone())
        .w_full()
        .h(40.0)
        .px(8.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.md)
        .hover_motion(rune::Anim::INSTANT, rune::Anim::INSTANT)
        .child(
            col()
                .grow(1.0)
                .shrink()
                .min_w(0.0)
                .child(
                    text(credential.name.clone())
                        .font(k.ty.ui_medium)
                        .color(k.c.text_base)
                        .truncate(),
                )
                .child(
                    text(credential.username.clone())
                        .font(k.ty.mono)
                        .color(k.c.text_muted)
                        .truncate(),
                ),
        )
        .child(text(users.to_string()).font(k.ty.small).color(k.c.text_faint))
        .on_press(move |s: &mut Bezel| s.select_credential(&id));
    item = k.interactive(item);
    if selected {
        item = item.fill(k.c.pressed);
    }
    item
}

fn credential_detail(sheet: &Credentials, saved: Option<&Credential>, snapshot: &Snapshot, k: &Kit) -> El {
    let input = |edit: &TextEdit, label: &'static str, field: fn(&mut Credentials) -> &mut TextEdit| {
        TextInput::new(edit)
            .label(label)
            .width(None)
            .on_edit(move |s: &mut Bezel| match &mut s.sheet {
                Some(Sheet::Credentials(sheet)) => field(sheet),
                _ => &mut s.host,
            })
            .on_submit(|s: &mut Bezel, _: &mut Tx| s.save_credential())
    };
    let name = input(&sheet.name, "Name", |c| &mut c.name)
        .placeholder("Domain admin")
        .build(k)
        .key("credential-name");
    let username = input(&sheet.username, "Username", |c| &mut c.username)
        .placeholder(r"DOMAIN\user, user@domain or user")
        .build(k)
        .key("credential-username");
    let password = input(&sheet.password, "Password", |c| &mut c.password)
        .placeholder(if saved.is_some() { "Type a new password" } else { "" })
        .secure()
        .build(k)
        .key("credential-password");
    let mut password_field = col().w_full().gap(4.0).child(field(k, "Password", password));
    if let Some(updated) = saved.and_then(|c| c.password_updated) {
        password_field = password_field.child(
            text(updated_ago(updated, today()))
                .font(k.ty.caption())
                .color(k.c.text_muted)
                .key("credential-updated"),
        );
    }
    let mut detail = col()
        .key("credential-detail")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .pl(16.0)
        .gap(16.0)
        .child(field(k, "Name", name))
        .child(field(k, "Username", username))
        .child(password_field);
    if let Some(credential) = saved {
        let users = snapshot.used_by(&credential.id);
        let mut used = col().w_full().gap(4.0).child(
            text(match users.len() {
                0 => "Used by no connections".to_owned(),
                1 => "Used by 1 connection".to_owned(),
                n => format!("Used by {n} connections"),
            })
            .font(k.ty.small_medium)
            .color(k.c.text_muted),
        );
        for user in &users {
            used = used.child(
                row()
                    .gap(6.0)
                    .center_y()
                    .child(rune::icon(&icons::MONITOR, 14.0).color(k.c.icon_faint))
                    .child(text(user.name.clone()).font(k.ty.ui).color(k.c.text_base).truncate()),
            );
        }
        detail = detail.child(
            col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(Default::default())
                .child(used),
        );
    } else {
        detail = detail.child(el().grow(1.0));
    }
    if let Some(message) = &sheet.error {
        detail = detail.child(error(k, message));
    }
    let saving = sheet.saving();
    let save = Button::new(match (saving, saved.is_some()) {
        (true, _) => "Saving…",
        (false, true) => "Save",
        (false, false) => "Add credential",
    })
    .variant(ButtonVariant::Contrast)
    .disabled(saving)
    .on_press(Bezel::save_credential)
    .build(k)
    .key("credential-save");
    let mut actions = row().w_full().gap(8.0).center_y();
    if let Some(credential) = saved {
        let id = credential.id.clone();
        actions = actions.child(
            Button::new("Delete…")
                .variant(ButtonVariant::Ghost)
                .on_press(move |s: &mut Bezel| s.ask_delete(Delete::Credential(id.clone())))
                .build(k)
                .key("credential-delete"),
        );
    }
    detail.child(actions.child(el().grow(1.0)).child(save))
}

/// "Password updated today", "… yesterday" or "… n days ago".
pub fn updated_ago(updated: time::Date, today: time::Date) -> String {
    match (today - updated).whole_days() {
        ..=0 => "Password updated today".into(),
        1 => "Password updated yesterday".into(),
        n => format!("Password updated {n} days ago"),
    }
}

fn certificates(snapshot: &Snapshot, k: &Kit, layer: &mut Vec<El>) {
    let mut list = col().key("certificates-list").role("list").w_full().gap(8.0);
    if snapshot.trusted_certificates.is_empty() {
        list = list.child(
            text("No trusted certificates. Bezel adds one when you tick Remember this certificate.")
                .font(k.ty.ui)
                .color(k.c.text_faint)
                .wrap()
                .w_full(),
        );
    }
    for certificate in &snapshot.trusted_certificates {
        let (host, port, sha256) = (certificate.host.clone(), certificate.port, certificate.sha256);
        list = list.child(
            col()
                .key(format!("certificate-{host}:{port}"))
                .role("listitem")
                .w_full()
                .gap(8.0)
                .padding(12.0)
                .radius(k.radius.md)
                .fill(k.c.layer_02)
                .border(1.0, k.c.border_weak)
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(
                            text(format!("{host}:{port}"))
                                .font(k.ty.mono)
                                .color(k.c.text_base)
                                .grow(1.0)
                                .shrink()
                                .truncate(),
                        )
                        .child(
                            Button::new("Remove")
                                .variant(ButtonVariant::Ghost)
                                .control_size(ControlSize::Small)
                                .on_press(move |s: &mut Bezel| s.forget_certificate(host.clone(), port, sha256))
                                .build(k),
                        ),
                )
                .child(fact_el(k, "SHA-256", fingerprint_text(k, &grouped(certificate.sha256))))
                .child(fact(k, "Issued by", certificate.issuer.clone()))
                .child(fact(k, "Expires", date(certificate.not_after)))
                .child(fact(k, "Remembered", date(certificate.remembered))),
        );
    }
    let body = col()
        .w_full()
        .shrink()
        .min_h(0.0)
        .px(16.0)
        .pb(16.0)
        .scroll(Default::default())
        .child(list);
    Dialog::new("certificates", "Trusted certificates")
        .description("Bezel connects to these hosts without asking while they present the same certificate.")
        .open(true)
        .fit()
        .width(560.0)
        .on_close(Bezel::close_sheet)
        .child(body)
        .build(k, layer);
}

pub fn confirm(s: &Bezel, k: &Kit, layer: &mut Vec<El>) {
    let (Some(delete), Some(snapshot)) = (&s.deleting, s.library.ready()) else {
        return;
    };
    let (title, description, action) = match delete {
        Delete::Connection(id) => {
            let name = snapshot.connection(id).map_or("this connection", |c| c.name.as_str());
            (format!("Delete {name}?"), String::new(), "Delete connection")
        }
        Delete::Group(id) => {
            let name = snapshot.group(id).map_or("this group", |g| g.name.as_str());
            let count = snapshot
                .connections
                .iter()
                .filter(|c| c.group.as_ref() == Some(id))
                .count();
            let description = match count {
                0 => String::new(),
                1 => "Its connection moves to Ungrouped.".into(),
                n => format!("Its {n} connections move to Ungrouped."),
            };
            (format!("Delete {name}?"), description, "Delete group")
        }
        Delete::Credential(id) => {
            let name = snapshot.credential(id).map_or("this credential", |c| c.name.as_str());
            let description = match snapshot.used_by(id).len() {
                0 => "No connections use it. Its password is removed from the Keychain.".into(),
                1 => "1 connection uses it and will ask for a username and password each time. Its password is removed from the Keychain.".into(),
                n => format!(
                    "{n} connections use it and will ask for a username and password each time. Its password is removed from the Keychain."
                ),
            };
            (format!("Delete {name}?"), description, "Delete credential")
        }
    };
    let body = col().w_full().px(16.0).pb(16.0).child(buttons(
        Button::new("Cancel")
            .on_press(Bezel::cancel_delete)
            .build(k)
            .key("confirm-cancel")
            .default_button(),
        Button::new(action)
            .variant(ButtonVariant::Danger)
            .on_press(Bezel::delete)
            .build(k)
            .key("confirm-delete"),
    ));
    let mut dialog = Dialog::new("confirm", title)
        .open(true)
        .fit()
        .width(400.0)
        .on_close(Bezel::cancel_delete)
        .child(body);
    if !description.is_empty() {
        dialog = dialog.description(description);
    }
    dialog.build(k, layer);
}

#[cfg(test)]
mod tests {
    use time::macros::date;

    use super::*;

    #[test]
    fn says_how_long_ago_the_password_changed() {
        let today = date!(2026 - 11 - 25);
        assert_eq!(updated_ago(today, today), "Password updated today");
        assert_eq!(updated_ago(date!(2026 - 11 - 24), today), "Password updated yesterday");
        assert_eq!(
            updated_ago(date!(2026 - 08 - 23), today),
            "Password updated 94 days ago"
        );
    }
}
