# The IR keymap, and how the test screen uses it

`ir-remote-test` shows what the @sgc seat delivers. What it delivers depends on
one file on the board — `/etc/rc_keymaps/x98h.toml` — plus the unit that loads
it. This guide is the whole procedure: why the keymap is needed at all, how to
install it, how to extend it, and what the screen then shows.

## Why a keymap is needed

Two independent gaps sit in front of the remote:

1. **The device has no keys without a map.** On the X98H/Z8Pro dtb the sunxi-ir
   node (`ir@7040000`, pin PH10) is `status = "okay"`, and the driver comes up
   (`rc1: sunxi-ir`, `/dev/lirc0`, "initialized sunXi IR driver"). But the DT
   carries no `linux,rc-map-name`, and `sunxi-cir` then uses `RC_MAP_EMPTY`: raw
   IR is received, and **no key event is ever produced**. `ir-keytable` is not
   in the image either, so there is nothing to map it with out of the box.
2. **@sgc only advertises devices it classifies.** The daemon enumerates
   `/dev/input/event*` and calls `classify()` (`resource_manager/input.rs`). Its
   keyboard rule wants at least one **typing key, ESC..Space (1..=57)** —
   deliberately, so power buttons and hotkey arrays stay out of the inventory.
   A TV remote reports navigation/media codes only (this one starts at
   `KEY_HOME` = 102), so it is skipped with `Skipping … not a mouse/keyboard/
   touch` and no client can ever acquire it.

The map below closes the first gap, and one extra entry closes the second.

## Files

| file | installed to | what it is |
| --- | --- | --- |
| `keymap/x98h.toml` | `/etc/rc_keymaps/x98h.toml` | the 22 buttons of the X98H/Z8Pro remote, plus one typing-key entry for the classifier |
| `packaging/ir-keymap.service` | `/etc/systemd/system/ir-keymap.service` | applies that map at boot on `rc1` |
| this guide | — | install, extend, port to another remote |

## Install on a board

    apt install ir-keytable                                   # not in the image
    install -Dm644 keymap/x98h.toml            /etc/rc_keymaps/x98h.toml
    install -Dm644 packaging/ir-keymap.service /etc/systemd/system/ir-keymap.service
    systemctl daemon-reload
    systemctl enable --now ir-keymap.service

The unit is a `oneshot` that waits up to 30 s for `/dev/lirc0`, then runs
`ir-keytable -s rc1 -c -w /etc/rc_keymaps/x98h.toml`: clear the table, write
this one. Writing also narrows the device's enabled protocols to those in the
file (`nec`, plus `lirc` for raw reads).

Verify the keymap itself:

    ir-keytable -s rc1 -r | head        # the table the kernel now holds
    cat /sys/class/rc/rc1/uevent        # NAME= is the active map name
    ls /dev/lirc0                       # raw IR channel

`rc1` is the IR receiver on these boards; **`rc0` is the HDMI-CEC receiver**
(`dw_hdmi`), and confusing the two sends you chasing a device that was never the
remote.

## The typing key, and why it is in the map

`0x9f7ff = "KEY_ESC"` is the second gap's workaround. Device capabilities come
from the map, so one entry in ESC..Space is enough for `classify()` to call the
receiver a keyboard — and `0x9f7ff` is a scancode this remote never sends, so no
button changes behaviour. Then tell the daemon to re-probe `/dev/input`:

    systemctl restart simple-graphics-controller
    journalctl -u simple-graphics-controller -b -o cat | grep "Opened /dev/input"

    INFO resource_manager::input: Opened /dev/input/event0 (dw_hdmi): Input(Keyboard(0))
    INFO resource_manager::input: Opened /dev/input/event1 (sunxi-ir): Input(Keyboard(1))

Two alternatives, and the honest one is not this line:

- **Map a real button into the range** (`0x9f71c = "KEY_ESC"` for BACK, the
  LibreELEC convention). Honest — the device really can send ESC — but it
  changes what BACK does for every consumer.
- **Widen `classify()`** so a device whose keys are consumer/navigation codes is
  admitted on its merits. That is the real fix; the nudge can go the moment the
  daemon carries it.

## Run the test screen against it

    just build-gnu-aarch64                     # host: see the project README
    scp target/aarch64-unknown-linux-gnu/release/ir-remote-test <board>:/root/
    # on the board, with the @sgc daemon running and no dashboard holding the seat:
    RUST_LOG=info /root/ir-remote-test --raw

`--raw` adds the diagnostic evdev read-back (auto-detected by the name fragment
`sunxi`, or pass a path). What the two layers show for this remote:

- `raw` rows name **every** button (`POWER`, `OK`, `MUTE`, `VOL±`, `CH±`,
  `TV`, `SETUP`, `MODE`, `ASSISTANT`, the four colour keys, arrows, `Home`,
  `Menu`, `Back`).
- `seat` rows identify only the buttons whose xkb keysym carries a character:
  arrows (`U+F700..F703`) and `Home` (`U+F729`) in practice, plus
  `Menu`/`Escape`/`Return`/printables when the map produces those keysyms.
  Everything else arrives as **`U+0000`** — the event is delivered, but nameless,
  so several buttons look identical there.
- A colour key (`KEY_RED/GREEN/YELLOW/BLUE`) has no character; `KEY_F13..F16`
  does (`U+F710..U+F713`). If a Slint consumer has to tell the four launch
  buttons apart, use F13–F16 in the map. A raw-evdev consumer
  (libsgc-rs/libsgc-c, LVGL's evdev indev) does not care.

## Extending the map

    systemctl stop ir-keymap.service
    ir-keytable -s rc1 -p all          # narrow nothing: discovery wants all decoders
    ir-keytable -s rc1 -t              # press buttons, read protocol + scancode

Then append `0x<scancode> = "KEY_<name>"` to the file and
`systemctl restart ir-keymap` (the clear-and-write the unit does is what makes a
changed file take effect). Check the name first if unsure — a keycode that the
kernel does not know (`grep -E '^#define KEY_' /usr/include/linux/input-event-codes.h`)
is rejected at load.

- **Remapping an existing scancode** needs nothing else: consumers read the
  keycode off the event.
- **Adding a new keycode** changes what the device advertises, and a consumer
  that cached the device's capabilities (libinput in the Slint backend does, at
  device add) keeps the old view — restart the test screen after such an edit.
- The device index in the resource name (`Input(Keyboard(1))`) is assigned per
  class by the daemon; it is not stable across reboots, so read it from the
  journal or the app's log rather than hard-coding it.

## Porting to another remote

The scancodes are **specific to this remote model** — another remote on the same
box is a fresh mapping job. The method that worked here, and the reason to trust
the result:

1. Press every button twice in one announced order, with `ir-keytable -s rc1 -t`
   capturing to a file; the log gives protocol + scancode per press.
2. Write a provisional map from the press order, load it, and sweep again in a
   **different** announced order. Because a loaded map makes the log print the
   key *name* a scancode currently carries, a wrong guess shows up as the wrong
   name under a known press, and a button skipped in the first sweep shows up as
   a code that never appeared.
3. Two sweeps that agree are the evidence; that is how the 22 entries above were
   confirmed (see the project README for the seat/raw measurement).

If the remote is a Bluetooth or 2.4 GHz "voice" remote it never appears under
`/sys/class/rc` at all — check `/sys/class/bluetooth` and `lsusb` before
spending time on a map.
