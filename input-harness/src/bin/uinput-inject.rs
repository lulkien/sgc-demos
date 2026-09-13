//! Create a virtual input device via /dev/uinput and inject events on command.
//!
//! Why a uinput device and not a probe of a real one: the @sgc daemon enumerates
//! `/dev/input` ONCE at startup (input has no hot-plug), and a uinput device
//! exists only while the process holding `/dev/uinput` is alive — so starting
//! this BEFORE the daemon makes it a normal, grantable device, and the whole
//! test can run without a human at the board. Commands arrive on stdin so one
//! process holds the device for a whole run and injects at the exact moment the
//! test needs: before a steal, and again after the re-grant. That last injection
//! is what answers "do events still reach the app after a revoke/re-grant?" —
//! the device being back in libinput is not the same thing.
//!
//! Usage: uinput-inject <keyboard|mouse|touch> [name]
//!
//! Prints `ready <syspath> <name>` once the device exists (identify the device
//! by NAME in the daemon's and the app's logs — the /dev/input/eventN index
//! depends on what else is attached), then `ack <command>` per command:
//!
//!     key <NAME>[+<NAME>...]   press the chord, sync, release it reversed, sync
//!     tap <NAME>               one key down + up
//!     press <NAME>             hold a key down (release it explicitly later) —
//!     release <NAME>           …which is how a modifier is held ACROSS a steal:
//!                              the held-down state is what a revoke has to drop
//!     move <dx> <dy>           relative move (mouse only)
//!     click                    BTN_LEFT down + up (mouse only)
//!     touch <x> <y>            ABS_X/ABS_Y + BTN_TOUCH down, then up (touch only)
//!     quit                     exit (destroys the device); EOF does the same
//!
//! Key names: the ones this harness needs (LEFTCTRL, LEFTALT, BACKSPACE, ESC,
//! ENTER, SPACE, TAB, LEFT, RIGHT, UP, DOWN, A..E) or a raw evdev code number.
//! Needs root and the uinput module loaded (`modprobe uinput`).

use std::io::{BufRead, Write};

use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, EventType, InputEvent, KeyCode, RelativeAxisCode,
    SynchronizationCode, UinputAbsSetup, uinput::VirtualDevice,
};

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let kind = args.next().unwrap_or_else(|| "keyboard".into());
    let name = args.next().unwrap_or_else(|| format!("sgc-virtual-{kind}"));

    let mut device = build(&kind, &name)?;
    let syspath = device.get_syspath()?;
    println!("ready {} {name}", syspath.display());
    std::io::stdout().flush()?;

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match dispatch(&mut device, line) {
            Ok(ack) => println!("ack {ack}"),
            Err(err) => println!("err {line}: {err}"),
        }
        std::io::stdout().flush()?;
        if line == "quit" {
            break;
        }
    }
    Ok(())
}

/// A `SYN_REPORT`: libinput only consumes a batch of events once it is
/// terminated by one, and a press+release in one batch reads as a tap.
fn syn() -> InputEvent {
    InputEvent::new(
        EventType::SYNCHRONIZATION.0,
        SynchronizationCode::SYN_REPORT.0,
        0,
    )
}

fn key_event(code: u16, value: i32) -> InputEvent {
    InputEvent::new(EventType::KEY.0, code, value)
}

fn rel_event(code: u16, value: i32) -> InputEvent {
    InputEvent::new(EventType::RELATIVE.0, code, value)
}

fn abs_event(code: u16, value: i32) -> InputEvent {
    InputEvent::new(EventType::ABSOLUTE.0, code, value)
}

fn dispatch(device: &mut VirtualDevice, line: &str) -> Result<String, String> {
    let mut parts = line.split_whitespace();
    let command = parts.next().unwrap_or_default();
    match command {
        "key" | "tap" => {
            let chord = parts.next().ok_or("expected <NAME>[+<NAME>...]")?;
            let keys = chord
                .split('+')
                .map(parse_key)
                .collect::<Result<Vec<_>, _>>()?;
            let mut events: Vec<InputEvent> =
                keys.iter().map(|key| key_event(key.code(), 1)).collect();
            events.push(syn());
            events.extend(keys.iter().rev().map(|key| key_event(key.code(), 0)));
            events.push(syn());
            device.emit(&events).map_err(|e| e.to_string())?;
            Ok(format!("{command} {chord}"))
        }
        "press" | "release" => {
            let name = parts.next().ok_or("expected <NAME>")?;
            let key = parse_key(name)?;
            let value = i32::from(command == "press");
            let events = [key_event(key.code(), value), syn()];
            device.emit(&events).map_err(|e| e.to_string())?;
            Ok(format!("{command} {name}"))
        }
        "move" => {
            let dx: i32 = parts
                .next()
                .ok_or("expected <dx>")?
                .parse()
                .map_err(|e| format!("{e}"))?;
            let dy: i32 = parts
                .next()
                .ok_or("expected <dy>")?
                .parse()
                .map_err(|e| format!("{e}"))?;
            let events = [
                rel_event(RelativeAxisCode::REL_X.0, dx),
                rel_event(RelativeAxisCode::REL_Y.0, dy),
                syn(),
            ];
            device.emit(&events).map_err(|e| e.to_string())?;
            Ok(format!("move {dx} {dy}"))
        }
        "click" => {
            let events = [
                key_event(KeyCode::BTN_LEFT.code(), 1),
                syn(),
                key_event(KeyCode::BTN_LEFT.code(), 0),
                syn(),
            ];
            device.emit(&events).map_err(|e| e.to_string())?;
            Ok("click".into())
        }
        "touch" => {
            let x: i32 = parts
                .next()
                .ok_or("expected <x>")?
                .parse()
                .map_err(|e| format!("{e}"))?;
            let y: i32 = parts
                .next()
                .ok_or("expected <y>")?
                .parse()
                .map_err(|e| format!("{e}"))?;
            let down = [
                abs_event(AbsoluteAxisCode::ABS_X.0, x),
                abs_event(AbsoluteAxisCode::ABS_Y.0, y),
                key_event(KeyCode::BTN_TOUCH.code(), 1),
                syn(),
            ];
            device.emit(&down).map_err(|e| e.to_string())?;
            let up = [key_event(KeyCode::BTN_TOUCH.code(), 0), syn()];
            device.emit(&up).map_err(|e| e.to_string())?;
            Ok(format!("touch {x} {y}"))
        }
        "quit" => Ok("quit".into()),
        other => Err(format!("unknown command {other}")),
    }
}

/// The capabilities decide how the daemon classifies the device (see
/// `resource_manager::input::classify`): Touch needs ABS_X + ABS_Y, Mouse needs
/// REL_X + REL_Y plus a BTN_MOUSE button, Keyboard needs any code in 1..=57.
/// Declaring the wrong set silently yields a device the daemon skips.
fn build(kind: &str, name: &str) -> std::io::Result<VirtualDevice> {
    match kind {
        "keyboard" => {
            let mut keys = AttributeSet::<KeyCode>::new();
            for key in [
                KeyCode::KEY_ESC,
                KeyCode::KEY_BACKSPACE,
                KeyCode::KEY_ENTER,
                KeyCode::KEY_SPACE,
                KeyCode::KEY_LEFTCTRL,
                KeyCode::KEY_LEFTALT,
                KeyCode::KEY_TAB,
                KeyCode::KEY_LEFT,
                KeyCode::KEY_RIGHT,
                KeyCode::KEY_UP,
                KeyCode::KEY_DOWN,
                KeyCode::KEY_A,
                KeyCode::KEY_B,
                KeyCode::KEY_C,
                KeyCode::KEY_D,
                KeyCode::KEY_E,
            ] {
                keys.insert(key);
            }
            VirtualDevice::builder()?
                .name(name)
                .with_keys(&keys)?
                .build()
        }
        "mouse" => {
            let mut keys = AttributeSet::<KeyCode>::new();
            keys.insert(KeyCode::BTN_LEFT);
            keys.insert(KeyCode::BTN_RIGHT);
            let mut axes = AttributeSet::<RelativeAxisCode>::new();
            axes.insert(RelativeAxisCode::REL_X);
            axes.insert(RelativeAxisCode::REL_Y);
            VirtualDevice::builder()?
                .name(name)
                .with_keys(&keys)?
                .with_relative_axes(&axes)?
                .build()
        }
        "touch" => {
            let mut keys = AttributeSet::<KeyCode>::new();
            keys.insert(KeyCode::BTN_TOUCH);
            let range = || AbsInfo::new(0, 0, 4095, 0, 0, 0);
            VirtualDevice::builder()?
                .name(name)
                .with_keys(&keys)?
                .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_X, range()))?
                .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_Y, range()))?
                .build()
        }
        other => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("unknown device kind {other:?}: keyboard|mouse|touch"),
        )),
    }
}

/// Key names the harness needs, plus a raw evdev code escape hatch.
fn parse_key(name: &str) -> Result<KeyCode, String> {
    let upper = name.trim().to_ascii_uppercase();
    let code = match upper.as_str() {
        "LEFTCTRL" | "CTRL" => KeyCode::KEY_LEFTCTRL,
        "LEFTALT" | "ALT" => KeyCode::KEY_LEFTALT,
        "BACKSPACE" => KeyCode::KEY_BACKSPACE,
        "ESC" | "ESCAPE" => KeyCode::KEY_ESC,
        "ENTER" => KeyCode::KEY_ENTER,
        "SPACE" => KeyCode::KEY_SPACE,
        "TAB" => KeyCode::KEY_TAB,
        "LEFT" => KeyCode::KEY_LEFT,
        "RIGHT" => KeyCode::KEY_RIGHT,
        "UP" => KeyCode::KEY_UP,
        "DOWN" => KeyCode::KEY_DOWN,
        "A" => KeyCode::KEY_A,
        "B" => KeyCode::KEY_B,
        "C" => KeyCode::KEY_C,
        "D" => KeyCode::KEY_D,
        "E" => KeyCode::KEY_E,
        other => {
            let code: u16 = other.parse().map_err(|_| format!("unknown key {name:?}"))?;
            KeyCode::new(code)
        }
    };
    Ok(code)
}
