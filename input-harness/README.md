# input-harness

Test tooling for the @sgc **input** path: it answers "does input still work after
a revoke/re-grant?" without a human touching the board.

Two reasons it exists:

- **A device being back in libinput is not proof that events flow.** The client
  logs its revoke/re-grant handling, but only an injected key that the app reacts
  to shows the whole path is live again. That gap is what this closes.
- **Input has no hot-plug** — the daemon enumerates `/dev/input` once at startup —
  and a uinput device lives only while the process holding `/dev/uinput` is alive.
  So the virtual device must be created BEFORE the daemon starts; after that it is
  granted like any real device, and the test needs no hardware.

## Tools

| file | what it does |
| --- | --- |
| `src/bin/uinput-inject.rs` | Creates a virtual keyboard/mouse/touchscreen via `/dev/uinput` and injects events read from stdin (`key LEFTCTRL+LEFTALT+BACKSPACE`, `tap A`, `move dx dy`, `click`, `touch x y`, `quit`) |
| `src/bin/sgc-steal.rs` | Takes one @sgc resource for N ms and lets go: FairQueue makes that preempt the app, then the daemon re-grants the queued owner — the revoke/re-grant driver |
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

1. stops the dashboard unit (one DRM lease, so the test client needs the screen),
   starts the injector plus a FIFO holder that keeps it from seeing EOF;
2. restarts the daemon so it enumerates the virtual device and prints its
   resource (`Opened /dev/input/event6 (sgc-virtual-keyboard): Input(Keyboard(2))`);
3. starts the client and reads the resource index back out of its log;
4. **negative control**: injects a plain `A` — the client must survive, which is
   what makes the later exit mean something;
5. steals that keyboard for 2 s and asserts the client logged the revoke AND the
   re-add of the device, and is still alive;
6. injects Ctrl+Alt+Backspace and asserts the client is gone.

Then it cleans up: kills the injector, restarts the daemon (so it forgets the
virtual device instead of holding a stale fd), and starts the dashboard unit.

## Observed (10.21.50.50, Pi 5, 2026-09-13)

    == 2/6 Opened /dev/input/event6 (sgc-virtual-keyboard): Input(Keyboard(2)) (fd 13)
    == 3/6 sgc-virtual-keyboard is Input(Keyboard(2))
    == 4/6 tap A — client survived
    == 5/6 revoked → re-granted → re-added — client still alive
    == 6/6 client exited: input works again after the revoke/re-grant
    == PASS

## Limits

- The injected chord is a **keyboard** signal. `mouse` and `touch` devices can be
  created and injected (`move`, `click`, `touch`) but nothing yet asserts that the
  app consumed those events — the cursor moving is visible, not assertable.
- One resource at a time: the driver steals only the virtual keyboard. Stealing
  several inputs at once, or inputs while the DRM lease is revoked too, is not
  covered.
- Needs the daemon restart in step 2 (see "no hot-plug" above) — that is a
  property of the daemon today, not of the harness.
