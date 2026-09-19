//! The optional raw-evdev read-back.
//!
//! The window's own key events come from the devices the daemon granted, and
//! the backend drops any keysym that has no character (most consumer/media
//! keys). To tell "the remote never sent it" from "the Slint path dropped it",
//! this panel reads the evdev node straight off `/dev/input`, which is a
//! DIAGNOSTIC - a real application must take its devices from @sgc, because the
//! daemon is what arbitrates who holds the seat.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use anyhow::{Context, Result};

/// Bytes in one `struct input_event` on 64-bit Linux: timeval(16) + type(2) +
/// code(2) + value(4). The timestamps are not needed, so only the tail is read.
const EVENT_LEN: usize = 24;
const EV_KEY: u16 = 1;

/// What to read: nothing, the auto-detected IR receiver, or a named node.
#[derive(Clone, Debug)]
pub enum Mode {
    Off,
    Auto,
    Device(String),
}

impl Mode {
    /// `--raw` (auto-detect) or `--raw <path-or-name-fragment>`.
    pub fn from_args(args: &[String]) -> Self {
        let Some(flag) = args.iter().position(|arg| arg == "--raw") else {
            return Self::Off;
        };
        match args.get(flag + 1) {
            Some(value) if !value.starts_with('-') => Self::Device(value.clone()),
            _ => Self::Auto,
        }
    }
}

/// One key event read from the device.
#[derive(Debug)]
pub struct RawKey {
    pub code: u16,
    pub value: i32,
}

/// A running read-back: which node it opened and where its events arrive.
pub struct Reader {
    pub path: PathBuf,
    pub events: Receiver<RawKey>,
}

/// Start reading, or return `None` when the read-back is off.
pub fn start(mode: &Mode) -> Result<Option<Reader>> {
    if matches!(mode, Mode::Off) {
        return Ok(None);
    }
    let path = match mode {
        Mode::Off => unreachable!("checked above"),
        Mode::Auto => find("sunxi")?,
        Mode::Device(spec) if Path::new(spec).exists() => PathBuf::from(spec),
        Mode::Device(spec) => find(spec)?,
    };

    let file = File::open(&path).with_context(|| format!("opening {}", path.display()))?;
    let (tx, events) = channel();
    std::thread::spawn(move || read_loop(file, tx));
    log::info!("raw read-back on {}", path.display());
    Ok(Some(Reader { path, events }))
}

/// Find an event node by a fragment of its device name (`/sys/class/input/.../name`).
fn find(wanted: &str) -> Result<PathBuf> {
    let mut seen = Vec::new();
    for entry in std::fs::read_dir("/sys/class/input").context("listing /sys/class/input")? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(node) = name.to_str().filter(|n| n.starts_with("event")) else {
            continue;
        };
        let device_name = std::fs::read_to_string(entry.path().join("device/name"))
            .unwrap_or_default()
            .trim()
            .to_string();
        if device_name.contains(wanted) {
            return Ok(PathBuf::from("/dev/input").join(node));
        }
        seen.push(format!("{node} = {device_name}"));
    }
    anyhow::bail!(
        "no input device whose name contains {wanted:?} (found: {})",
        seen.join(", ")
    )
}

fn read_loop(mut file: File, tx: Sender<RawKey>) {
    let mut buf = [0u8; EVENT_LEN];
    loop {
        if let Err(error) = file.read_exact(&mut buf) {
            log::warn!("raw read-back stopped: {error}");
            return;
        }
        let kind = u16::from_le_bytes([buf[16], buf[17]]);
        if kind != EV_KEY {
            continue;
        }
        let key = RawKey {
            code: u16::from_le_bytes([buf[18], buf[19]]),
            value: i32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]),
        };
        if tx.send(key).is_err() {
            return; // the UI is gone
        }
    }
}

/// Everything the reader produced since the last drain, newest last.
pub fn drain(events: &Receiver<RawKey>) -> Vec<RawKey> {
    let mut keys = Vec::new();
    loop {
        match events.try_recv() {
            Ok(key) => keys.push(key),
            Err(_) => return keys,
        }
    }
}
