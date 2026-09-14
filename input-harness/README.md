# input-harness

Test tooling for the @sgc **input** path: it answers "does input still work after
a revoke/re-grant, and after a device goes away and comes back?" without a human
touching the board.

Three reasons it exists:

- **A device being back in libinput is not proof that events flow.** The client
  logs its grant handling, but only an injected key that the app reacts to shows
  the whole path is live again. That gap is what this closes — with a negative
  control so the chord means something.
- **The daemon adopts a device created while it runs, and tells the clients that
  are already connected.** It reconciles `/dev/input` on every change (inotify),
  so the virtual device does not have to exist before the daemon starts, and it
  pushes the new list, so the client does not have to start after the device
  either: step 8 proves both halves with a device created under a running client.
- **A device that goes away is suspended, not revoked.** Killing the injector
  destroys the uinput device; the daemon must keep the resource for the client
  that holds it, and hand it back — same client, same name, no re-acquire — when
  a device of that class returns (steps 9 and 10).

## Tools

| file | what it does |
| --- | --- |
| `src/bin/uinput-inject.rs` | Creates a virtual keyboard/mouse/touchscreen via `/dev/uinput` and injects events read from stdin (`key LEFTCTRL+LEFTALT+BACKSPACE`, `tap A`, `move dx dy`, `click`, `touch x y`, `quit`) |
| `src/bin/sgc-steal.rs` | Takes one @sgc resource (`mouse:N`, `keyboard:N`, `touch:N`, `drm:N`) for N ms and lets go. With the seat rule an INPUT-only probe is denied a device the app's seat holds (step 5) — taking the DISPLAY is what preempts it (step 6) |
| `scripts/input-resume-test.sh` | Drives the whole sequence over ssh and asserts it (exit 0 = PASS) |

## Prerequisites

On the board:

    modprobe uinput                      # module; add uinput to /etc/modules-load.d/ to survive a reboot
    ls /dev/uinput                       # present in the image, but the module must be loaded

Copy the two tools to `/root` (they run as root: uinput and the @sgc socket):

    just build-gnu-aarch64
    scp target/aarch64-unknown-linux-gnu/release/{uinput-inject,sgc-steal} root@<board>:/root/

The client under test must be input-capable (`slint-lease-client --features input`,
or the dashboard built with `--features input`) and installed at `/root/lease-app`
by default (`APP=` overrides). The signal used by the driver is the backend's
Ctrl+Alt+Backspace quit: **the app disappearing proves the key arrived.**

## Run it

    ./scripts/input-resume-test.sh
    BOARD=root@10.21.50.51 APP=/root/adguard-slint ./scripts/input-resume-test.sh

What the driver does, in order:

1. restarts the daemon for a clean device table (suspension is in-memory state, so
   repeated runs would otherwise resume a name left over from the last one — the
   run itself still needs no restart, which is what step 2 asserts), stops the
   dashboard unit (one DRM lease, so the test client needs the screen), and starts
   the injector plus a FIFO holder that keeps it from seeing EOF;
2. asserts the RUNNING daemon took the virtual device (no restart during the run)
   and reads its devnode path out of the log
   (`Opened /dev/input/event6 (sgc-virtual-keyboard): Input(Keyboard(2)) (fd 14, plugged in while running)`);
3. starts the client and reads the resource index back out of its log;
4. **negative control**: injects a plain `A` — the client must survive, which is
   what makes the later exit mean something;
5. asserts an input-only probe is DENIED the keyboard the app's seat holds
   (`only the app on the display can take it`) — input follows the seat;
6. takes the DISPLAY with `sgc-steal drm:1`, which preempts the app: its lease and
   its devices are revoked with its seat, and the display is handed back when the
   probe exits (`re-granted (fd 6) — rebuilding display stack`);
7. asserts the client re-acquired its devices BY ITSELF when the display came back
   (`acquiring Input(Keyboard(2)) from @sgc (the display is back)`, then the device
   back in libinput) — in the same process, which is what the pid comparison is
   for: the engine hands the display back but never the devices, so re-asking is
   the client's half of the handover;
8. creates a SECOND virtual device (a mouse) while the client is up and asserts the
   client asked for it, was granted it and handed it to libinput — the daemon
   pushes its list whenever a device appears, which is how an app that is already
   running sees one;
9. kills the injector (which destroys the uinput device) and asserts the daemon
   SUSPENDED the resource: it names the device that went away, the client reports
   that the device is gone but the grant is kept, and no `Revoked` reached it;
10. starts the injector again (a new devnode, the same device name) and asserts the
    daemon RESUMED the resource for the SAME client (`no re-acquire`), the client
    handed it back to libinput — and that the client did not `acquire` it, which is
    the whole point — then injects Ctrl+Alt+Backspace on that device and asserts the
    client is gone.

Then it cleans up: kills the injector (the daemon suspends the vanished virtual
device on its own), and starts the dashboard unit.

## Observed (10.21.50.50, Pi 5, 2026-09-14)

    == 2/10 the RUNNING daemon takes the virtual device (hot-plug, no restart)
    2026-09-14T05:19:12Z  INFO ...::hotplug: Opened /dev/input/event6 (sgc-virtual-keyboard): Input(Keyboard(2)) (fd 14, plugged in while running)
    == 3/10 sgc-virtual-keyboard is Input(Keyboard(2))
    == 4/10 tap A — client survived
    == 5/10 ... WARN ...::engine: [client 3] denied Input(Keyboard(2)): Input(Keyboard(2)) is held by client 2 — only the app on the display can take it
    == 6/10 linuxsgc: lease Drm { card: 1 } revoked — suspending until re-granted
           linuxsgc: input: Input(Keyboard(2)) revoked — device removed from libinput
           linuxsgc: lease Drm { card: 1 } re-granted (fd 6) — rebuilding display stack
    == 7/10 linuxsgc: acquiring Input(Keyboard(2)) from @sgc (the display is back)...
           same process, devices re-acquired and re-added to libinput
    == 8/10 2026-09-14T05:21:15Z  INFO ...::hotplug: Opened /dev/input/event7 (sgc-late-mouse): Input(Mouse(1)) (fd 19, plugged in while running)
           linuxsgc: acquiring Input(Mouse(1)) from @sgc (appeared while running)...
           linuxsgc: input: libinput device added: Input(Mouse(1)) at /dev/input/event7 (sgc-late-mouse)
    == 9/10 2026-09-14T05:21:21Z  WARN ...::hotplug: Suspended Input(Keyboard(2)) (/dev/input/event6): the device is gone; its holder keeps it and is told when it is back
           2026-09-14T05:21:21Z  INFO ...::engine: Suspended Input(Keyboard(2)): client 5 keeps it until the device is back
           linuxsgc: input: Input(Keyboard(2)) (event6) was removed by the kernel — dropped from libinput; the grant is kept and the device comes back to this client
    == 10/10 2026-09-14T05:21:27Z  INFO ...::hotplug: Resumed Input(Keyboard(2)) (/dev/input/event6 (sgc-virtual-keyboard), fd 16): the device is back with its holder — no re-acquire
            linuxsgc: input: libinput device added: Input(Keyboard(2)) at /dev/input/event6 (sgc-virtual-keyboard)
            client exited: events flow again on the resumed device
    == PASS

The daemon was NOT restarted during the run, and step 8's device did not exist
when the client connected: the daemon adopted it, pushed the new list, and the
client — already running — acquired it and handed it to libinput. The whole run is
ONE client process: it re-acquires its devices when the display comes back (step
7), and holds `Input(Keyboard(2))` across that device's absence and return (steps
9 and 10) with no `acquiring` line for it.

## Limits

- The injected chord is a **keyboard** signal. `mouse` and `touch` devices can be
  created and injected (`move`, `click`, `touch`) but nothing yet asserts that the
  app consumed those events — the cursor moving is visible, not assertable.
- One resource at a time: the driver asserts one device's suspension, and takes
  the display to exercise the seat rule. Several inputs at once, or a device
  re-enumerated to a DIFFERENT device name (a "different device on the same node"
  replacement), is not covered.
- A seat handover is covered only for the devices the app already held: a device
  that is REFUSED before the handover is re-asked on the next one, but the driver
  does not assert that path.
- A device name left suspended by an aborted run stays reserved for the daemon's
  lifetime, which is why the driver restarts the daemon in its preflight.
