//! Bezel, an RDP client for connecting from a Mac to Windows machines.

mod app;
mod keychain;
mod keymap;
mod manage;
mod palette;
mod platform;
mod pointer;
mod scenes;
mod sheets;
mod sidebar;
mod store;
mod tabs;
mod view;
mod windows;

use app::Bezel;
use bezel_core::keys::Shortcut;
use rune::{AppSpec, MenuItem, Tx};
use tracing::Level;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

fn main() {
    init_logging();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        ironrdp = bezel_rdp::IRONRDP_REVISION,
        "Bezel starting"
    );
    let spec = AppSpec::<Bezel> {
        min_size: Some((640.0, 420.0)),
        fonts: rune_theme_opencode::FONTS,
        families: rune_theme_opencode::FAMILIES,
        scenes: scenes::SCENES,
        setup: Some(Bezel::setup),
        poll: Some(Bezel::poll),
        on_key: Some(Bezel::on_key),
        ..AppSpec::new("Bezel", (1280.0, 800.0), Bezel::new, view::view)
    };
    // A headless render has one window and no turns, so its state is the shared one.
    if rune::is_headless() {
        return rune::run(spec);
    }
    platform::quiet_system_edit_items();
    let (home, husk, main_window) = windows::home();
    let mut app = rune::App::new(spec)
        .init(move || husk)
        .keep_running()
        .turn_scope(windows::enter, windows::leave)
        .on_close_request(windows::on_main_close_request)
        .on_quit_request(windows::on_quit_request)
        .on_reopen(windows::on_reopen)
        .on_window(move |window| *main_window.borrow_mut() = Some(window.clone()));
    for item in menu_items() {
        app = app.menu_item(item);
    }
    home.borrow_mut().main = Some(app.handle());
    app.run();
}

/// A Bezel shortcut in the menu bar: its title, key equivalent and what it does.
type Command = (&'static str, &'static str, Shortcut);

/// In the app menu, where every Mac app keeps it.
const SETTINGS: Command = ("Settings…", "cmd+comma", Shortcut::Settings);

/// The Session menu, `None` for a separator.
const SESSION: [Option<Command>; 7] = [
    Some(("Connect…", "cmd+k", Shortcut::Palette)),
    None,
    Some(("Show Previous Tab", "shift+cmd+bracketleft", Shortcut::PreviousTab)),
    Some(("Show Next Tab", "shift+cmd+bracketright", Shortcut::NextTab)),
    None,
    Some(("Toggle Sidebar", "ctrl+cmd+s", Shortcut::Sidebar)),
    Some(("Toggle Full Screen", "ctrl+cmd+f", Shortcut::FullScreen)),
];

/// Bezel's menu items, beside the Edit and Window items every Rune app gets. Each lets the
/// window see its keys first (`keys_first`), so over a Session's desktop the Session's keyboard
/// takes the chord and nothing of it reaches Windows; a click runs it.
fn menu_items() -> impl Iterator<Item = MenuItem<Bezel>> {
    let item = |(title, keys, shortcut): Command| {
        MenuItem::with_action(title, move |s: &mut Bezel, tx: &mut Tx| {
            s.shortcut(shortcut, tx);
        })
        .key_equivalent(keys.parse().expect("a valid key equivalent"))
        .keys_first()
    };
    std::iter::once(item(SETTINGS)).chain(SESSION.into_iter().map(move |entry| match entry {
        Some(command) => item(command).menu("Session"),
        None => MenuItem::separator().menu("Session"),
    }))
}

/// A daily log in the platform log folder, keeping a week. IronRDP and the libraries under it
/// log warnings only: their debug output can carry protocol detail Bezel doesn't vet for
/// secrets, and Bezel's own logging never includes passwords.
fn init_logging() {
    let _ = std::fs::create_dir_all(platform::log_dir());
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("bezel")
        .filename_suffix("log")
        .max_log_files(7)
        .build(platform::log_dir());
    let filter = Targets::new()
        .with_default(Level::WARN)
        .with_target("bezel", Level::INFO);
    match appender {
        Ok(appender) => {
            let file = tracing_subscriber::fmt::layer().with_writer(appender).with_ansi(false);
            tracing_subscriber::registry().with(filter).with(file).init();
        }
        Err(e) => {
            eprintln!("Bezel can't write its log to {}: {e}", platform::log_dir().display());
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer())
                .init();
        }
    }
}

#[cfg(test)]
mod tests {
    use bezel_core::keys::{CommandKey, KeyOutcome, Keyboard, Scancode};

    use super::*;

    /// Every menu key equivalent is the Bezel shortcut its item runs, so the menus claim no key
    /// the Session should have.
    #[test]
    fn menu_keys_are_the_shortcuts_they_run() {
        for (title, keys, shortcut) in std::iter::once(SETTINGS).chain(SESSION.into_iter().flatten()) {
            let parsed: rune::Shortcut = keys.parse().unwrap();
            let mods = parsed.modifiers();
            let mut keyboard = Keyboard::new(CommandKey::MacShortcuts);
            for (held, key) in [
                (mods.shift, Scancode::SHIFT_LEFT),
                (mods.ctrl, Scancode::CONTROL_LEFT),
                (mods.alt, Scancode::ALT_LEFT),
                (mods.logo, Scancode::WINDOWS_LEFT),
            ] {
                if held {
                    keyboard.key(key, true);
                }
            }
            let key = keymap::scancode(parsed.physical_key()).unwrap();
            assert_eq!(keyboard.key(key, true), KeyOutcome::Shortcut(shortcut), "{title}");
        }
    }
}
