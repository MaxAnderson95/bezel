//! Bezel, an RDP client for connecting from a Mac to Windows machines.

mod app;
mod keychain;
mod keymap;
mod manage;
mod platform;
mod pointer;
mod scenes;
mod sheets;
mod sidebar;
mod store;
mod view;

use app::Bezel;
use rune::AppSpec;
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
    rune::run(AppSpec::<Bezel> {
        min_size: Some((640.0, 420.0)),
        fonts: rune_theme_opencode::FONTS,
        families: rune_theme_opencode::FAMILIES,
        scenes: scenes::SCENES,
        setup: Some(Bezel::setup),
        poll: Some(Bezel::poll),
        on_key: Some(Bezel::on_key),
        ..AppSpec::new("Bezel", (1280.0, 800.0), Bezel::new, view::view)
    });
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
