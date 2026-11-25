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
