//! The window: a deep titlebar band with the status line, and the raised panel holding the
//! connect box, the remote desktop and the dialogs. Every colour, type style and radius comes
//! from the kit.

use bezel_core::trust::{Certificate, Fingerprint, Question};
use bezel_rdp::{Ended, Input, MouseButton};
use rune::{Cx, El, Phase as DragPhase, PointerButton, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, Checkbox, ControlSize, Dialog, Field, Kit, Spinner, TextInput};
use rune_theme_opencode::icons;

use crate::app::{Bezel, Library, Live, Phase, Prompt, Screen, SignIn, Trust};
use crate::pointer;

const TITLEBAR: f32 = 40.0;
/// The bezel: the gap between the panel's edge and the remote desktop.
const INSET: f32 = 4.0;

pub fn view(s: &Bezel, cx: &mut Cx<Bezel>) -> El {
    let k = rune_theme_opencode::kit(cx.system_dark());
    let scale = cx.env().scale;
    let mut layer = Vec::new();
    let content = match &s.screen {
        Screen::Idle => idle(s, k),
        Screen::SignIn(sign_in) => {
            sign_in_dialog(sign_in, k, &mut layer);
            idle(s, k)
        }
        Screen::Session(live) => {
            if let Some(prompt) = &live.certificate {
                certificate_dialog(prompt, k, &s.library, &mut layer);
            }
            let mut stage = col().w_full().h_full().child(session(s, live, k, scale));
            if let Some(message) = &s.notice {
                stage = stage.child(
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
            stage
        }
    };
    let area = col()
        .key("area")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .on_drawn_layout_tx(|s: &mut Bezel, rect, tx: &mut Tx| s.layout_area(rect, tx))
        .child(content);
    let panel = k
        .elevate(
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
        .child(area);
    let window = col()
        .w_full()
        .h_full()
        .fill(k.c.bg_deep)
        .child(titlebar(s, k, cx.titlebar_inset()))
        .child(col().w_full().grow(1.0).min_h(0.0).px(8.0).pb(8.0).child(panel));
    rune_kit::root(window, layer)
}

fn titlebar(s: &Bezel, k: &Kit, inset: f32) -> El {
    let mut bar = row()
        .key("titlebar")
        .w_full()
        .h(TITLEBAR)
        .pl(inset + 12.0)
        .pr(12.0)
        .gap(12.0)
        .center_y()
        .window_drag();
    let Screen::Session(live) = &s.screen else {
        return bar.child(text("Bezel").font(k.ty.ui_medium).color(k.c.text_muted));
    };
    let (dot, state) = match live.state {
        Phase::Connected { width, height } => {
            let retina = s.area.is_some_and(|a| a.scale > 1.0);
            (
                k.c.success.icon,
                format!("{width} × {height}{}", if retina { " Retina" } else { "" }),
            )
        }
        Phase::Connecting => (k.c.warning.icon, "Connecting…".into()),
        Phase::Reconnecting => (k.c.warning.icon, "Reconnecting…".into()),
    };
    bar = bar
        .child(el().size(8.0, 8.0).radius(4.0).fill(dot).a11y_hidden())
        .child(
            text(format!("{}:{}", live.address.host, live.address.port))
                .font(k.ty.mono)
                .color(k.c.text_base),
        )
        .child(text(live.username.clone()).font(k.ty.ui).color(k.c.text_muted))
        .child(text(state).font(k.ty.ui).color(k.c.text_muted));
    if matches!(live.state, Phase::Connected { .. }) && live.trust == Trust::Verified {
        bar = bar.child(
            row()
                .gap(4.0)
                .center_y()
                .child(icon(&icons::SHIELD, 16.0).color(k.c.icon_muted))
                .child(text("Trusted").font(k.ty.ui).color(k.c.text_muted)),
        );
    }
    bar.child(el().grow(1.0)).child(
        Button::new("Disconnect")
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Small)
            .on_press(Bezel::disconnect)
            .build(k),
    )
}

fn idle(s: &Bezel, k: &Kit) -> El {
    let host = TextInput::new(&s.host)
        .label("Host")
        .placeholder("Type a host name to connect")
        .leading_icon(k.glyph.search)
        .large()
        .width(None)
        .invalid(s.host_error.is_some())
        .on_edit(|s: &mut Bezel| &mut s.host)
        .on_change(|s: &mut Bezel, _: &mut Tx| s.host_error = None)
        .on_submit(|s: &mut Bezel, _: &mut Tx| s.submit_host())
        .build(k)
        .key("host");
    let mut column = col().w_full().max_w(440.0).gap(12.0).child(host);
    if let Some(error) = &s.host_error {
        column = column.child(text(error.clone()).font(k.ty.ui).color(k.c.danger.fg));
    }
    if let Some(ended) = &s.ended {
        column = column.child(ended_card(ended, k));
    }
    if let Library::Failed(error) = &s.library {
        column = column.child(notice(
            k,
            error,
            "Bezel won't remember certificates until the file is fixed.",
            true,
        ));
    }
    if let Some(error) = &s.notice {
        column = column.child(notice(k, error, "", false));
    }
    col().w_full().h_full().center().padding(24.0).child(column)
}

fn ended_card(ended: &Ended, k: &Kit) -> El {
    let details = ended.details.clone();
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
                .child(
                    Button::new("Copy details")
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Small)
                        .on_press_tx(move |_: &mut Bezel, tx: &mut Tx| tx.copy(details.clone()))
                        .build(k),
                ),
        )
}

fn notice(k: &Kit, message: &str, detail: &str, open_folder: bool) -> El {
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

fn sign_in_dialog(sign_in: &SignIn, k: &Kit, layer: &mut Vec<El>) {
    let username = Field::new()
        .label("Username")
        .control(
            TextInput::new(&sign_in.username)
                .label("Username")
                .placeholder(r"DOMAIN\user, user@domain or user")
                .width(None)
                .on_edit(|s: &mut Bezel| match &mut s.screen {
                    Screen::SignIn(sign_in) => &mut sign_in.username,
                    _ => &mut s.host,
                })
                .on_submit(|s: &mut Bezel, _: &mut Tx| s.sign_in())
                .build(k)
                .key("sign-in-username"),
        )
        .build(k);
    let password = Field::new()
        .label("Password")
        .control(
            TextInput::new(&sign_in.password)
                .label("Password")
                .width(None)
                .secure()
                .invalid(sign_in.error.is_some())
                .on_edit(|s: &mut Bezel| match &mut s.screen {
                    Screen::SignIn(sign_in) => &mut sign_in.password,
                    _ => &mut s.host,
                })
                .on_submit(|s: &mut Bezel, _: &mut Tx| s.sign_in())
                .build(k)
                .key("sign-in-password"),
        )
        .build(k);
    let mut body = col()
        .w_full()
        .gap(16.0)
        .px(16.0)
        .pb(16.0)
        .child(username)
        .child(password);
    if let Some(error) = &sign_in.error {
        body = body.child(text(error.clone()).font(k.ty.ui).color(k.c.danger.fg).wrap().w_full());
    }
    body = body.child(buttons(
        Button::new("Cancel").on_press(Bezel::cancel_sign_in).build(k),
        Button::new("Connect")
            .variant(ButtonVariant::Contrast)
            .on_press(Bezel::sign_in)
            .build(k)
            .key("sign-in-connect")
            .default_button(),
    ));
    Dialog::new("sign-in", format!("Sign in to {}", sign_in.address.host))
        .open(true)
        .fit()
        .width(400.0)
        .on_close(Bezel::cancel_sign_in)
        .child(body)
        .build(k, layer);
}

fn buttons(secondary: El, primary: El) -> El {
    row()
        .w_full()
        .gap(8.0)
        .pt(4.0)
        .child(el().grow(1.0))
        .child(secondary)
        .child(primary)
}

fn certificate_dialog(prompt: &Prompt, k: &Kit, library: &Library, layer: &mut Vec<El>) {
    let host = &prompt.request.host;
    // Without its saved certificates Bezel can't tell a new certificate from a changed one,
    // and can't save one either.
    let unreadable = matches!(library, Library::Failed(_));
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
                        .on_toggle(Bezel::toggle_remember)
                        .build(k)
                        .key("certificate-remember"),
                )
                .child(buttons(
                    Button::new("Cancel").on_press(Bezel::refuse_certificate).build(k),
                    Button::new("Trust and connect")
                        .variant(ButtonVariant::Contrast)
                        .on_press(Bezel::trust)
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
                    .on_press(|s: &mut Bezel| s.confirm_change(false))
                    .build(k)
                    .default_button(),
                Button::new("Trust new certificate")
                    .variant(ButtonVariant::Danger)
                    .on_press(Bezel::trust)
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
                    .on_press(|s: &mut Bezel| s.confirm_change(true))
                    .build(k),
                Button::new("Don't connect")
                    .on_press(Bezel::refuse_certificate)
                    .build(k)
                    .default_button(),
            ));
            Dialog::new("certificate", format!("{host} has a different certificate"))
                .description("Someone may be intercepting the connection, or the server's certificate was replaced.")
                .child(body)
        }
    };
    dialog
        .open(true)
        .fit()
        .width(480.0)
        .on_close(Bezel::refuse_certificate)
        .build(k, layer);
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

fn fact(k: &Kit, label: &str, value: String) -> El {
    fact_el(k, label, text(value).font(k.ty.ui).color(k.c.text_base).wrap())
}

fn fact_el(k: &Kit, label: &str, value: El) -> El {
    row()
        .w_full()
        .gap(12.0)
        .items_start()
        .child(text(label.to_owned()).font(k.ty.ui).color(k.c.text_muted).w(96.0))
        .child(value.grow(1.0).shrink())
}

fn fingerprint_text(k: &Kit, fingerprint: &str) -> El {
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

fn date(date: time::Date) -> String {
    format!("{} {}, {}", date.month(), date.day(), date.year())
}

/// Groups of four hex digits, as fingerprints are read aloud and compared.
fn grouped(fingerprint: Fingerprint) -> String {
    let hex = fingerprint.to_string();
    hex.as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" ")
}

fn session(s: &Bezel, live: &Live, k: &Kit, scale: f32) -> El {
    let Some(area) = s.area else { return el() };
    let frame = match live.state {
        Phase::Connected { width, height } => Some((width, height)),
        Phase::Connecting | Phase::Reconnecting => {
            let (width, height) = live.handle.framebuffer().size();
            (width > 0).then_some((width as u16, height as u16))
        }
    };
    let Some(frame) = frame else {
        return col()
            .w_full()
            .h_full()
            .center()
            .gap(12.0)
            .child(Spinner::new().build(k))
            .child(
                text(format!("Connecting to {}…", live.address.host))
                    .font(k.ty.ui)
                    .color(k.c.text_muted),
            );
    };
    let [width, height] = desktop_size([area.width, area.height], frame, scale);
    let mut desktop = el()
        .key("desktop")
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
        .on_mount(|_: &mut Bezel, tx: &mut Tx| tx.focus("desktop", false))
        .on_pointer_move_local(move |s: &mut Bezel, p, tx: &mut Tx| {
            s.input(Input::PointerMove {
                x: p.x / width,
                y: p.y / height,
            });
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
            s.input(Input::PointerMove {
                x: d.x / width,
                y: d.y / height,
            });
            match d.phase {
                DragPhase::Down => {
                    tx.focus("desktop", false);
                    s.input(Input::Button { button, pressed: true });
                }
                DragPhase::Up => s.input(Input::Button { button, pressed: false }),
                DragPhase::Move => tx.live_only(),
            }
        })
        .on_wheel(|s: &mut Bezel, e, tx: &mut Tx| {
            s.input(Input::Wheel { dx: e.dx, dy: e.dy });
            tx.live_only();
            true
        })
        .on_physical_input(|s: &mut Bezel, e, _: &mut Tx| s.physical(e));
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
