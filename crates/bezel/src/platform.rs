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

/// `~/Library/Application Support/Bezel` on macOS, the folder `bezel.toml` lives in.
pub fn data_dir() -> PathBuf {
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

/// Clears every menu key equivalent that would take a key from the Session: winit's Hide and
/// Hide Others (⌘H, ⌥⌘H) and AppKit's Close All (⌥⌘W). Kept: Bezel's own items and Quit, whose
/// keys are Bezel's, and Rune's Edit and Window items, which hand their keys to the window
/// first. AppKit keeps its window tiling items, full screen and Minimize All outside the menu's
/// items, so those keep their keys, each needing the Globe key or ⌥⌘M. True once the menu bar
/// exists and has been handled, so the caller can stop asking.
#[cfg(target_os = "macos")]
pub fn release_menu_keys() -> bool {
    use objc2::runtime::Sel;
    use objc2::{MainThreadMarker, sel};
    use objc2_app_kit::{NSApplication, NSMenu};
    use objc2_foundation::NSString;

    fn keeps(action: Sel) -> bool {
        [
            sel!(runeMenuAction:),
            sel!(terminate:),
            sel!(undo:),
            sel!(redo:),
            sel!(cut:),
            sel!(copy:),
            sel!(paste:),
            sel!(selectAll:),
            sel!(performMiniaturize:),
            sel!(performClose:),
        ]
        .contains(&action)
    }

    fn release(menu: &NSMenu) {
        for i in 0..menu.numberOfItems() {
            let Some(item) = menu.itemAtIndex(i) else { continue };
            if let Some(submenu) = item.submenu() {
                release(&submenu);
            }
            if item.keyEquivalent().length() > 0 && !item.action().is_some_and(keeps) {
                item.setKeyEquivalent(&NSString::new());
            }
        }
    }

    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(menu) = NSApplication::sharedApplication(mtm).mainMenu() else {
        return false;
    };
    release(&menu);
    true
}

#[cfg(not(target_os = "macos"))]
pub fn release_menu_keys() -> bool {
    true
}

/// Asks AppKit not to manage the Edit menu's Start Dictation and Emoji & Symbols items, or it
/// gives Emoji & Symbols ⌃⌘Space again whenever it updates the menu, after `release_menu_keys`
/// cleared it. AppKit reads these defaults as it builds the menu, so this runs before the app
/// starts.
#[cfg(target_os = "macos")]
pub fn quiet_system_edit_items() {
    use objc2_foundation::{NSString, NSUserDefaults};

    let defaults = NSUserDefaults::standardUserDefaults();
    for key in ["NSDisabledDictationMenuItem", "NSDisabledCharacterPaletteMenuItem"] {
        defaults.setBool_forKey(true, &NSString::from_str(key));
    }
}

#[cfg(not(target_os = "macos"))]
pub fn quiet_system_edit_items() {}

/// The Mac clipboard's plain text, or `None` when it holds none. Only the text is read, so an
/// image on the clipboard is never converted; AppKit allows it off the main thread.
#[cfg(target_os = "macos")]
pub fn clipboard_text() -> Option<String> {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::NSString;

    autoreleasepool(|_| {
        // NSPasteboardTypeString.
        let plain = NSString::from_str("public.utf8-plain-text");
        let text = NSPasteboard::generalPasteboard().stringForType(&plain)?.to_string();
        Some(text).filter(|t| !t.is_empty())
    })
}

#[cfg(not(target_os = "macos"))]
pub fn clipboard_text() -> Option<String> {
    None
}
