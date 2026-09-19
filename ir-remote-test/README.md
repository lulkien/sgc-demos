# ir-remote-test

A **Slint** test screen for an IR remote on a board running
`simple-graphics-controller` (`@sgc`). It holds the seat (a DRM lease plus the
input devices the daemon granted) and shows every key the stack delivers:

    header   IR remote test - keys delivered by the @sgc seat
    panel    the last key: its glyph or codepoint, its name, press/release
    log      the last 10 events, newest first, tagged by source
    footer   which layers are live; Ctrl+Alt+Backspace quits

It is a test tool, not an application: the window knows nothing about `@sgc`.
The linuxsgc backend owns the session (connect, lease, devices, revoke,
re-grant) and hands the app ordinary Slint key events, so what is on screen is
exactly what an app would receive.

## Two layers, because they disagree

| source | what it is | where it comes from |
| --- | --- | --- |
| `seat` | Slint key events | the granted evdev devices → libinput → xkb → `ui/main.slint` |
| `raw` | evdev keycodes | the node is read directly (diagnostic, `--raw`) |

The `raw` layer exists because the seat path is lossy in a way that is easy to
mistake for "the remote is dead". The backend maps the xkb keysym through
`i_slint_common::for_each_keys!` and falls back to
`char::from_u32(keysym_to_utf32(sym))` (`calloop_backend/input.rs`,
`map_key_sym`). A keysym with no character does NOT drop the event — `from_u32(0)`
is a valid `char` — so the press arrives carrying **U+0000**, with no identity at
all: several different buttons are then indistinguishable.

Measured on the X98H/Z8Pro remote (one sweep over every button, seat layer:
204 events, raw layer: 318):

- identifiable through the seat: the four arrows (`U+F700..F703`) and `Home`
  (`U+F729`) — plus `Menu`/`Escape`/`Return`/`Backspace`/printable ASCII and
  `F13..F16` (`U+F710..F713`) when the keymap actually produces those keysyms
- `U+0000`, i.e. visible but nameless: `POWER`, `OK`, `MUTE`, `VOL±`, `CH±`,
  `TV`, `SETUP`, `MODE`, `ASSISTANT`, and the colour keys (`KEY_RED/GREEN/
  YELLOW/BLUE`)
- the raw layer names every one of them by keycode

Practical consequence for a keymap: a media button's keysym is what decides
whether the framework path can identify it. Mapping the four app buttons to
`F13..F16` makes them identifiable as `U+F710..U+F713`; mapping them to
`KEY_RED/GREEN/YELLOW/BLUE` makes them arrive as `U+0000`. For a program that
reads keycodes itself (libsgc-rs/libsgc-c, or LVGL's evdev indev) it makes no
difference.

Reading evdev directly is a **diagnostic**, not a pattern to copy: a real
application takes its devices from `@sgc`, because the daemon is what
arbitrates who holds the seat.

## The remote is not advertised yet

`@sgc` builds its input inventory from `/dev/input/event*` and classifies each
device (`resource_manager/input.rs`, `classify`). Its keyboard rule wants at
least one **typing key, ESC..Space (1..=57)** — deliberately, so power buttons
and hotkey arrays stay out — and a TV remote reports navigation/media codes
only (the remote's lowest code is `KEY_HOME` = 102). So the device is dropped
with `Skipping … not a mouse/keyboard/touch` and never reaches a client.

Two ways to get it into the inventory:

1. **Keymap (no code change).** `ir-keytable` capabilities come from the map, so
   one entry in that range makes the device a `Keyboard(n)` for the daemon. This
   project ships that map and the unit that loads it:

   - `keymap/x98h.toml` → `/etc/rc_keymaps/x98h.toml` (22 buttons + the one
     typing-key entry)
   - `packaging/ir-keymap.service` → `/etc/systemd/system/ir-keymap.service`

   The full procedure — install, verify, extend, port to another remote — is
   `docs/ir-keymap.md`. In short:

       # honest: BACK really does send ESC afterwards
       0x9f71c = "KEY_ESC"

       # or semantics-free: the scancode this remote never sends (what ships)
       0x9f7ff = "KEY_ESC"
   and then `systemctl restart simple-graphics-controller` so the daemon
   re-probes `/dev/input`.

2. **`classify()` widening (the real fix).** Teach the daemon that a device
   whose keys are consumer/navigation codes is a keyboard too, then the remote
   is `Input(Keyboard(n))` like any other and every consumer path (libsgc-rs/C,
   LVGL, Slint) works unchanged.

## Build and run

    just build-gnu-aarch64            # the board flavor: femtovg (GPU) + input
    just                              # host build, same feature set

GPU only, on purpose: the fork's linuxsgc backend survives a lease revoke on the
GL path (it rebuilds its EGL/GBM stack on the fresh lease fd), so the software
renderer is not compiled in — a CPU fallback would only hide a broken GL
userspace behind a slow screen.

On the board (root, and the @sgc daemon must be running — a client is
sgc-or-die):

    ./ir-remote-test                   # seat layer only
    ./ir-remote-test --raw             # + auto-detected IR receiver evdev node
    ./ir-remote-test --raw /dev/input/event1

`--raw` without a value looks for an input device whose name contains `sunxi`;
with a value it takes a path or a name fragment. Every key event is also logged
(`RUST_LOG=info`, one line per key), so a headless run is readable over ssh:
`journalctl -f` or the app's stdout.

Only one client can hold the lease: stop the dashboards before running this,
and remember that Ctrl+Alt+Backspace (handled in the backend) is the way out.
If the screen stays dark or the window is the wrong size, the backend sizes
itself from the lease's mode — `SLINT_DRM_MODE=3` (1080p60) is the workaround
for an odd EDID.

## Pitfalls

- A **denied** input acquire is permanent for the process; a **queued** one is
  not. If this screen shows the key panel but no events, check the daemon's
  journal for the acquire lines before suspecting the remote.
- The device's own key bitmap and the delivered keycode can disagree after a
  keymap reload (`ir-keytable -c -w`); the press that is delivered follows the
  keytable, which is what the seat layer shows.
- `input` is in this project's **default** features on purpose (unlike the
  dashboards): the app exists to consume the granted devices, so a build
  without it would hold the lease and test nothing. It is gnu-only — libinput
  cannot be linked into a musl static build.
