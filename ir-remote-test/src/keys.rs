//! Naming and bookkeeping for the key events both layers report.
//!
//! A Slint key event carries TEXT, not a keycode: the linuxsgc backend maps the
//! xkb keysym through `i_slint_common::for_each_keys!` and DROPS the event when
//! the keysym has no character (that is what happens to most consumer/media
//! keysyms, e.g. XF86AudioMute). What arrives is therefore either a printable
//! character or one of the private-use placeholders Slint uses for named keys
//! (arrows are U+F700.., F13 is U+F710, Home is U+F729, Menu U+F735).

use std::rc::Rc;

use slint::{Model, ModelRc, VecModel};

use crate::{KeyRow, MainWindow};

/// How many log rows the screen shows at once. The rows are a glance, not a
/// scrollback - the journal carries the whole history (one line per key), and a
/// bounded model keeps this project free of ListView/std-widgets.
const VISIBLE_ROWS: usize = 10;

/// One event, already named for the screen.
struct Entry {
    source: &'static str,
    symbol: String,
    name: String,
    detail: String,
}

/// The rolling log plus the last-key summary, owned by the UI thread.
pub struct KeyLog {
    model: Rc<VecModel<KeyRow>>,
    seq: i32,
    presses: i32,
}

impl KeyLog {
    pub fn new() -> Self {
        Self {
            model: Rc::new(VecModel::default()),
            seq: 0,
            presses: 0,
        }
    }

    /// The model the window renders; pushing into it repaints the list.
    pub fn model(&self) -> ModelRc<KeyRow> {
        ModelRc::from(self.model.clone())
    }

    /// Report a key event the window received through the @sgc seat.
    pub fn on_seat_key(&mut self, ui: &MainWindow, text: &str, down: bool) {
        let name = symbol_name(text);
        // A keysym with no character arrives as U+0000, not as "no event": the
        // backend maps it through `for_each_keys!` and falls back to
        // `char::from_u32(keysym_to_utf32(sym))`, which yields NUL rather than
        // dropping the key. So the press is visible but carries no identity -
        // several different buttons look identical here.
        let note = if text.starts_with('\0') {
            "  (xkb has no character for this keysym - the raw rows name it)"
        } else {
            ""
        };
        let detail = format!(
            "{}  xkb text {}{note}",
            if down { "press" } else { "release" },
            codepoints(text)
        );
        if down {
            self.presses += 1;
        }
        log::info!(
            "seat key: {} ({}){}",
            name,
            codepoints(text),
            if down { "" } else { " [release]" }
        );
        self.push(
            ui,
            Entry {
                source: "seat",
                symbol: if text.is_empty() {
                    "(no text)".into()
                } else {
                    printable(text)
                },
                name,
                detail,
            },
        );
    }

    /// Report a key event the raw evdev panel read off the device.
    pub fn on_raw_key(&mut self, ui: &MainWindow, code: u16, value: i32) {
        let name = code_name(code);
        let what = match value {
            0 => "release",
            1 => "press",
            2 => "repeat",
            _ => "value",
        };
        log::info!("raw key: {name} (code {code}) [{what}]");
        self.push(
            ui,
            Entry {
                source: "raw",
                symbol: name.clone(),
                name: name.clone(),
                detail: format!("{what}  evdev code {code}"),
            },
        );
    }

    fn push(&mut self, ui: &MainWindow, entry: Entry) {
        self.seq += 1;
        // Newest first: the row under the key panel is always the last press.
        self.model.insert(
            0,
            KeyRow {
                seq: self.seq,
                source: entry.source.into(),
                symbol: entry.symbol.clone().into(),
                detail: entry.detail.clone().into(),
            },
        );
        while self.model.row_count() > VISIBLE_ROWS {
            let last = self.model.row_count() - 1;
            self.model.remove(last);
        }
        ui.set_last_symbol(entry.symbol.into());
        ui.set_last_name(entry.name.into());
        ui.set_last_detail(entry.detail.into());
        ui.set_count_label(format!("{} events, {} presses", self.seq, self.presses).into());
    }
}

/// A label for the keysym text Slint delivered: printable characters speak for
/// themselves, the private-use placeholders get the name Slint uses for them.
pub fn symbol_name(text: &str) -> String {
    match text.chars().next() {
        None => "(empty text)".to_string(),
        Some(c) => match c as u32 {
            0x00 => "no character (keysym unmapped)".into(),
            0x08 => "Backspace".into(),
            0x09 => "Tab".into(),
            0x0a => "Return".into(),
            0x1b => "Escape".into(),
            0x7f => "Delete".into(),
            0xf700 => "ArrowUp".into(),
            0xf701 => "ArrowDown".into(),
            0xf702 => "ArrowLeft".into(),
            0xf703 => "ArrowRight".into(),
            0xf70d..=0xf70f => format!("F{}", 10 + (c as u32 - 0xf70d)),
            0xf710..=0xf716 => format!("F{}", 13 + (c as u32 - 0xf710)),
            0xf729 => "Home".into(),
            0xf735 => "Menu".into(),
            0xf746 => "Help".into(),
            0xf748 => "Back".into(),
            _ => printable(text),
        },
    }
}

/// The text as it should be shown: a glyph when it can be drawn, otherwise the
/// codepoint (DejaVu has no glyphs in the private-use area the named keys use).
pub fn printable(text: &str) -> String {
    match text.chars().next() {
        Some(c) if (0x20..0xf000).contains(&(c as u32)) => c.to_string(),
        _ => codepoints(text),
    }
}

/// `U+F700` for one character, or a space-separated list when the event carried
/// more (an empty text shows as `-`).
pub fn codepoints(text: &str) -> String {
    if text.is_empty() {
        return "-".to_string();
    }
    text.chars()
        .map(|c| format!("U+{:04X}", c as u32))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Name an evdev keycode. The codes the X98H/Z8Pro remote sends are spelled
/// out; anything else falls back to `KEY_<n>` so the number is still readable.
pub fn code_name(code: u16) -> String {
    let named = match code {
        1 => "ESC",
        28 => "ENTER",
        102 => "HOME",
        103 => "UP",
        105 => "LEFT",
        106 => "RIGHT",
        108 => "DOWN",
        113 => "MUTE",
        114 => "VOLUMEDOWN",
        115 => "VOLUMEUP",
        116 => "POWER",
        139 => "MENU",
        141 => "SETUP",
        158 => "BACK",
        167 => "NEXTSONG",
        183..=186 => return format!("F{}", 13 + (code - 183)),
        352 => "OK",
        373 => "MODE",
        377 => "TV",
        398 => "RED",
        399 => "GREEN",
        400 => "YELLOW",
        401 => "BLUE",
        402 => "CHANNELUP",
        403 => "CHANNELDOWN",
        583 => "ASSISTANT",
        _ => return format!("KEY_{code}"),
    };
    named.to_string()
}
