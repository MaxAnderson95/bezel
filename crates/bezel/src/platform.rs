//! Where Bezel keeps its data and logs. Everything platform-specific lives here so the rest of
//! the app builds and tests on Linux too.

use std::path::PathBuf;

fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// `~/Library/Application Support/Bezel/bezel.toml` on macOS.
pub fn library_file() -> PathBuf {
    data_dir().join("bezel.toml")
}

/// `~/Library/Logs/Bezel` on macOS.
pub fn log_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        home().join("Library/Logs/Bezel")
    } else {
        xdg("XDG_STATE_HOME", ".local/state").join("bezel")
    }
}

fn data_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support/Bezel")
    } else {
        xdg("XDG_DATA_HOME", ".local/share").join("bezel")
    }
}

fn xdg(variable: &str, fallback: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(fallback))
}

/// Clears the key equivalents of the app menu's Hide and Hide Others, which winit installs, so
/// ⌘H and ⌥⌘H reach the Session; the spec reserves neither. True once the menu bar exists and
/// has been handled, so the caller can stop asking.
#[cfg(target_os = "macos")]
pub fn release_hide_shortcuts() -> bool {
    use objc2::{MainThreadMarker, sel};
    use objc2_app_kit::{NSApplication, NSMenu};
    use objc2_foundation::NSString;

    fn release(menu: &NSMenu) {
        for i in 0..menu.numberOfItems() {
            let Some(item) = menu.itemAtIndex(i) else { continue };
            if let Some(submenu) = item.submenu() {
                release(&submenu);
            }
            if matches!(item.action(), Some(action) if action == sel!(hide:) || action == sel!(hideOtherApplications:))
            {
                item.setKeyEquivalent(&NSString::new());
            }
        }
    }

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(menu) = app.mainMenu() else { return false };
    release(&menu);
    true
}

#[cfg(not(target_os = "macos"))]
pub fn release_hide_shortcuts() -> bool {
    true
}
