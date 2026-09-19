//! IR remote test screen on a @sgc seat, via the fork's linuxsgc backend.
//!
//! The backend owns the whole @sgc session (connect, lease, devices, revoke and
//! re-grant), so this app enables `backend-linuxsgc-libinput` and the backend
//! selector does the rest: the key events that reach the UI are ordinary Slint
//! key events, already normalized from the granted evdev devices through
//! libinput and xkb.
//!
//! Two things are worth knowing before reading the screen:
//!
//! - The daemon advertises `Input(...)` only for devices that look like a
//!   keyboard, mouse or touchscreen to its classifier, and its keyboard rule
//!   wants at least one typing key (ESC..Space). A TV remote reports navigation
//!   and media codes only, so it stays out of the inventory until either the
//!   daemon's classifier admits it or the keymap gives the device one key in
//!   that range. See the README for both routes.
//! - A key whose xkb keysym has no character never becomes a Slint key event
//!   (the backend drops it), so most media keys are invisible here. That is
//!   what the optional `--raw` panel is for: it reads the evdev node directly
//!   and shows the keycodes the stack above it dropped.

mod keys;
mod raw;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context, Result};

use keys::KeyLog;

slint::include_modules!();

/// How often the UI thread drains the raw read-back.
const UI_TICK_MS: u64 = 100;

const EMPTY_HINT: &str = "no key events yet. The screen shows what the @sgc seat delivers: \
the daemon must be advertising the remote as an Input device (see the README - its keyboard \
rule wants one typing key, which a TV remote's keymap does not have), and the device has to \
be granted to this app. With --raw, presses are also read straight off the evdev node, so a \
press that appears there but not above is the framework path dropping it.";

fn main() -> Result<()> {
    // RUST_LOG drives the level, defaulting to `info`: one line per delivered
    // key, which is rare enough to keep the journal readable.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let raw_mode = raw::Mode::from_args(&std::env::args().collect::<Vec<_>>());

    // Window creation first: it installs the linuxsgc backend (which connects to
    // @sgc and acquires the lease), and the font collection needs the global
    // context that the backend installs.
    let ui = MainWindow::new().context("creating the UI window: is the @sgc daemon running?")?;
    register_font();

    let log = Rc::new(RefCell::new(KeyLog::new()));
    ui.set_rows(log.borrow().model());
    ui.set_empty_hint(EMPTY_HINT.into());

    let reader = raw::start(&raw_mode)?;
    ui.set_mode_line(mode_line(reader.as_ref()).into());

    // Key events arrive on the event-loop thread; the seat is single-threaded,
    // so Rc<RefCell<..>> is the whole of the sharing this app needs.
    {
        let log = Rc::clone(&log);
        let weak = ui.as_weak();
        ui.on_key_event(move |text, down| {
            if let Some(ui) = weak.upgrade() {
                log.borrow_mut().on_seat_key(&ui, text.as_str(), down);
            }
        });
    }

    // The raw read-back is a thread, so its events are drained on a timer
    // instead of from a callback.
    let timer = slint::Timer::default();
    if let Some(reader) = reader {
        let weak = ui.as_weak();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(UI_TICK_MS),
            move || {
                let Some(ui) = weak.upgrade() else {
                    return;
                };
                for key in raw::drain(&reader.events) {
                    log.borrow_mut().on_raw_key(&ui, key.code, key.value);
                }
            },
        );
    }

    ui.run().context("event loop failed")?;
    Ok(())
}

/// The footer line: which layers are live and how to leave the app.
fn mode_line(reader: Option<&raw::Reader>) -> String {
    let raw = match reader {
        Some(reader) => format!("raw read-back: {}", reader.path.display()),
        None => "raw read-back off (--raw [device])".to_string(),
    };
    format!("seat: @sgc lease + granted devices | {raw} | Ctrl+Alt+Backspace quits")
}

/// Load a DejaVu font into the process-global fontique collection. The board
/// image ships neither fontconfig nor freetype, so the system font source is
/// empty and the text pipeline needs the file registered directly.
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
