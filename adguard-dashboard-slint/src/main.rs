//! AdGuard Home statistics dashboard as a Slint UI, on a DRM lease from the
//! simple-graphics-controller daemon (@sgc), via the fork's linuxsgc backend.
//!
//! The backend owns the whole @sgc session - connecting, acquiring the lease,
//! pumping it, and surviving a revoke by parking and resuming the display stack
//! - so this app never names the backend or SgcClient; it enables the
//! `backend-linuxsgc` feature and the backend selector does the rest. Both
//! renderer flavors survive a revoke: the CPU one re-inits its dumb-buffer
//! display, the femtovg/GL one drops and rebuilds its EGL/GBM stack on the
//! fresh lease fd.
//!
//! Data flows on a worker thread (fetch `/control/stats` and `/control/status`
//! every `refresh_secs`, push the outcome over a channel) while the UI thread
//! only drains that channel on a timer. A slow or failing request therefore
//! never blocks rendering, and a failed refresh keeps the last snapshot on
//! screen instead of blanking it.

mod agh;
mod config;
mod selfcheck;
mod view;

use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::Duration;

use anyhow::{Context, Result};

use agh::{FetchOutcome, Snapshot};
use config::Config;

slint::include_modules!();

/// How often the UI thread drains the channel and refreshes the status line.
const UI_TICK_MS: u64 = 200;

fn main() -> Result<()> {
    // RUST_LOG drives the level, defaulting to `info`: routine progress lines
    // (one per refresh) are `debug`, so the unit's RUST_LOG=info journal shows
    // only state changes and failures.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();
    let cfg = Config::load(&config::path_from_args(&args))?;

    // The self-check renders offscreen: no window, no lease, no daemon needed.
    // It takes the panel sizes to check (`--size WxH,...`) so it can be run
    // against the panel this board actually has.
    if args.iter().any(|arg| arg == "--self-check") {
        return selfcheck::run(&cfg, &selfcheck::sizes_from_args(&args)?);
    }

    // Window creation first: it installs the linuxsgc backend (which connects to
    // @sgc and acquires the lease), and the font collection needs the global
    // context that the backend installs.
    let ui = MainWindow::new().context("creating the UI window: is the @sgc daemon running?")?;
    register_font();

    let source = cfg.base_url.clone();
    ui.set_footer_source(source.clone().into());

    let refresh_secs = cfg.refresh_secs;
    let (tx, rx) = channel();
    std::thread::spawn(move || fetch_loop(cfg, tx));

    let ui_weak = ui.as_weak();
    let mut latest: Option<Snapshot> = None;
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(UI_TICK_MS),
        move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };

            if let Some(outcome) = drain(&rx) {
                match outcome {
                    FetchOutcome::Ok(snapshot) => {
                        view::fill(&ui, &snapshot, &source);
                        log::debug!(
                            "refreshed: {} queries, {} blocked",
                            snapshot.queries,
                            snapshot.blocked_total()
                        );
                        latest = Some(snapshot);
                    }
                    FetchOutcome::CredentialsRejected => {
                        log::warn!("refresh failed: credentials rejected");
                        view::set_error(&ui, "credentials rejected");
                    }
                    FetchOutcome::Failed(msg) => {
                        log::warn!("refresh failed: {msg}");
                        view::set_error(&ui, "stale");
                    }
                }
            }

            if let Some(snapshot) = &latest {
                view::set_age(&ui, snapshot.age_secs(), refresh_secs as f64 * 3.0);
            }
        },
    );

    ui.run().context("event loop failed")?;
    Ok(())
}

/// Fetch on the configured interval and push the outcome to the UI thread.
fn fetch_loop(cfg: Config, tx: Sender<FetchOutcome>) {
    loop {
        let outcome = agh::fetch(&cfg);
        if tx.send(outcome).is_err() {
            return; // the UI is gone
        }
        std::thread::sleep(Duration::from_secs(cfg.refresh_secs));
    }
}

/// Keep only the newest outcome received since the last tick.
fn drain(rx: &Receiver<FetchOutcome>) -> Option<FetchOutcome> {
    let mut latest = None;
    loop {
        match rx.try_recv() {
            Ok(outcome) => latest = Some(outcome),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
        }
    }
    latest
}

/// Load a DejaVu font into the process-global fontique collection. The board
/// image ships neither fontconfig nor freetype, so the system font source is
/// empty and the text pipeline needs the file registered directly (the same
/// trick slint-lease-client uses).
pub(crate) fn register_font() {
    const CANDIDATES: [&str; 3] = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
    ];

    for path in CANDIDATES {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        use slint::fontique_011::fontique;

        let blob = fontique::Blob::new(std::sync::Arc::new(bytes));
        let mut collection = slint::fontique_011::shared_collection();
        let count = collection.register_fonts(blob, None).len();
        log::info!("registered {count} font(s) from {path}");
        return;
    }
    log::warn!("no DejaVuSans.ttf found - text will need fontconfig");
}
