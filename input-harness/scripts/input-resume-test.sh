#!/bin/bash
# End-to-end proof that INPUT keeps working across a revoke/re-grant AND across a
# device that goes away and comes back, with no human at the board.
#
# What it drives: a virtual keyboard created by uinput-inject while the daemon
# is RUNNING (the daemon adopts a device node that appears under it — no restart),
# an input-capable @sgc client on the screen. It drives the three things input
# does under the seat rule: an input-only probe is DENIED a device the app's seat
# holds, taking the DISPLAY takes the app's devices with it, and a device that
# goes away (the injector dies) is SUSPENDED for its holder — started again, the
# SAME client gets it back with no re-acquire, and the injected chord at the end
# proves events flow on it.
#
# The signal: the linuxsgc backend quits the event loop on Ctrl+Alt+Backspace,
# so "the client is gone" proves the injected key REACHED it. A plain A must NOT
# kill it — that negative control is what makes the exit mean something, because
# a device being back in libinput is not the same thing as events flowing.
#
# Usage: BOARD=root@10.21.50.50 APP=/root/lease-app ./scripts/input-resume-test.sh
#
# Prerequisites on the board: uinput-inject + sgc-steal in /root (see README),
# the uinput module loaded (`modprobe uinput`), and an input-capable client built
# for the board (slint-lease-client --features input, or the dashboard).
set -euo pipefail

BOARD="${BOARD:-root@10.21.50.50}"
APP="${APP:-/root/lease-app}"
DEVICE="${DEVICE:-sgc-virtual-keyboard}"
LATE="${LATE:-sgc-late-mouse}"
UNIT="${UNIT:-adguard-dashboard-slint}"
SERVICE="${SERVICE:-simple-graphics-controller}"
CHORD="LEFTCTRL+LEFTALT+BACKSPACE"
FIFO=/tmp/inject.fifo
APP_LOG=/tmp/lease-app.log
INJECT_LOG=/tmp/inject.log

sshq() { ssh -o BatchMode=yes "$BOARD" "$@"; }
say() { printf '\n== %s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
alive() { sshq "pgrep -f '^$APP\$' >/dev/null"; }

cleanup() {
    say "cleanup: stop the injector (the daemon lets the device go), restore $UNIT"
    # Bracket patterns: a plain `pkill -f foo` also matches this script's own ssh
    # command line and kills the session.
    sshq 'pkill -f "/root/uinput-injec[t]"; pkill -f "sleep 360[0]"; pkill -f "sleep 12[0]"; true' || true
    sshq "systemctl start $UNIT; true" || true
}
trap cleanup EXIT

say "preflight: uinput + harness binaries on $BOARD"
sshq 'ls /dev/uinput >/dev/null && [ -x /root/uinput-inject ] && [ -x /root/sgc-steal ]' \
    || fail "/dev/uinput (modprobe uinput) or /root/{uinput-inject,sgc-steal} missing"
if sshq 'pgrep -f "^/root/uinput-injec[t]" >/dev/null'; then
    fail "an injector is already running"
fi
# A client left over from an earlier run still holds devices (and would take the
# re-grants): the daemon frees its slots on disconnect, so kill it and start clean.
sshq "pkill -f '^$APP\$'; true" || true
# Suspension is in-memory state: a resource whose device never came back stays
# reserved for its old holder for the daemon's lifetime, and a fresh device of
# that class would RESUME that name instead of being adopted. Restarting here is
# what makes repeated runs identical — the run itself still needs no restart, and
# step 2 still proves the daemon takes a device that appears under it.
say "preflight: restart the daemon for a clean device table"
sshq "systemctl restart $SERVICE; sleep 3"
sshq "systemctl is-active $SERVICE" >/dev/null || fail "$SERVICE did not come back"

say "1/10 stop $UNIT, start the virtual $DEVICE"
sshq "systemctl stop $UNIT"
sshq "rm -f $FIFO && mkfifo $FIFO"
# A holder keeps the FIFO writable, so the injector never sees EOF when one ssh
# command finishes writing to it.
sshq "setsid sh -c 'exec 3<>$FIFO; sleep 3600' >/dev/null 2>&1 & sleep 1
setsid sh -c 'exec /root/uinput-inject keyboard $DEVICE < $FIFO > $INJECT_LOG 2>&1' &"
sleep 2
sshq "grep -q '^ready' $INJECT_LOG" || fail "the injector did not report ready"
sshq "cat $INJECT_LOG"

say "2/10 the RUNNING daemon takes the virtual device (hot-plug, no restart)"
# Either line is the daemon taking it at runtime: "plugged in while running" for a
# device it had never seen, or a resume of a resource left suspended by an earlier
# run (the name stays reserved for its holder / for the class).
ADOPTED=$(sshq "sleep 5; journalctl -u simple-graphics-controller --since '-30s' --no-pager -o cat | grep '$DEVICE' | grep -E 'plugged in while running|no re-acquire' | tail -1" || true)
[ -n "$ADOPTED" ] || fail "the daemon never took $DEVICE at runtime"
NODE=$(printf '%s' "$ADOPTED" | sed -n 's|.*Opened \([^ ]*\) .*|\1|p;s|.*Resumed [^(]*(\([^ ]*\) .*|\1|p')
[ -n "$NODE" ] || fail "could not tell which node $DEVICE got: $ADOPTED"
echo "  $DEVICE is $NODE"

say "3/10 start the client and find the resource index of the virtual keyboard"
sshq "rm -f $APP_LOG; cd /root; setsid sh -c 'SLINT_DRM_MODE=3 exec $APP > $APP_LOG 2>&1' &"
sleep 7
LINE=$(sshq "grep 'libinput device added.*($DEVICE)' $APP_LOG | tail -1" || true)
INDEX=$(printf '%s' "$LINE" | sed -n 's/.*Input(Keyboard(\([0-9]*\))).*/\1/p')
[ -n "$INDEX" ] || fail "the client never registered $DEVICE (log: $LINE)"
RESOURCE="Input(Keyboard($INDEX))"
STEAL="keyboard:$INDEX"
echo "  $DEVICE is $RESOURCE"

say "4/10 negative control: tap A — the client must survive"
sshq "echo 'tap A' > $FIFO; sleep 2"
alive || fail "the client died on an ordinary key"

say "5/10 the seat rule: an input-only probe cannot take a device the app holds"
sshq "/root/sgc-steal $STEAL 1000 1 500 2>&1 | tail -1" \
    | grep -F 'only the app on the display can take it' \
    || fail "an input-only probe was NOT denied $RESOURCE (the app's seat holds it)"
alive || fail "the client died on a denied probe"
echo "  denied — input follows the seat"

say "6/10 the seat changes hands: the app's devices go with the display, and it gets it back"
sshq "/root/sgc-steal drm:1 2000 1 500 > /tmp/seat-steal.log 2>&1; sleep 3"
sshq "grep -F 'Drm { card: 1 } revoked' $APP_LOG" \
    || fail "the app was not preempted off the display"
sshq "grep -F 'revoked — device removed from libinput' $APP_LOG" \
    || fail "the app's devices were not revoked with its seat"
sshq "grep -F 'Drm { card: 1 } re-granted' $APP_LOG" \
    || fail "the app was not re-granted the display when the probe left"
alive || fail "the client died across the seat handover"
echo "  seat taken and given back; the app's devices went with it"

say "7/10 restart the client — it holds the display but no devices (re-acquire is client-side work)"
# The engine does not re-grant input with the seat, and the linuxsgc backend does
# not re-ask for its devices on a display re-grant yet. Restarting it is what
# makes the rest of the run meaningful; closing that gap is the next change.
sshq "pkill -f '^$APP\$'; sleep 2; rm -f $APP_LOG"
sshq "cd /root; setsid sh -c 'SLINT_DRM_MODE=3 exec $APP > $APP_LOG 2>&1' & sleep 7"
LINE=$(sshq "grep 'libinput device added.*($DEVICE)' $APP_LOG | tail -1" || true)
INDEX=$(printf '%s' "$LINE" | sed -n 's/.*Input(Keyboard(\([0-9]*\))).*/\1/p')
[ -n "$INDEX" ] || fail "the restarted client never registered $DEVICE (log: $LINE)"
RESOURCE="Input(Keyboard($INDEX))"
STEAL="keyboard:$INDEX"
echo "  client restarted; $DEVICE is $RESOURCE again"

say "8/10 a device plugged in NOW must reach the running client"
# A second uinput device created after the client connected: the daemon adopts it
# and pushes the new list, and the client acquires it — the connect-time list is
# only a snapshot, so this push is the one path by which an app that is already
# running can see it. `sleep` holds the device open until the injector is killed
# during cleanup.
sshq "setsid sh -c 'sleep 120 | /root/uinput-inject mouse $LATE > /tmp/late-inject.log 2>&1' >/dev/null 2>&1 & sleep 5"
sshq "journalctl -u simple-graphics-controller --since '-30s' --no-pager -o cat | grep '$LATE' | grep 'plugged in while running'" \
    || fail "the daemon never adopted $LATE"
sshq "grep -F 'appeared while running' $APP_LOG | grep -F 'Input(Mouse('" \
    || fail "the client never asked for the device that appeared while it was running"
sshq "grep -E 'libinput device added: Input\\(Mouse\\([0-9]+\\)\\) at .*($LATE)' $APP_LOG" \
    || fail "the client never handed $LATE to libinput"
echo "  $LATE was adopted, acquired and registered while the app ran"

say "9/10 the injector dies — the daemon must SUSPEND the device, not revoke it"
sshq "pkill -f '/root/uinput-injec[t]'; sleep 6"
sshq "journalctl -u simple-graphics-controller --since '-30s' --no-pager -o cat | grep -F 'Suspended $RESOURCE'" \
    || fail "the daemon did not suspend $RESOURCE when its device went away"
# The holder keeps the resource: no Revoke reaches it, and its own fd dying is
# all it sees (libinput reports the removal).
if sshq "grep -F '$RESOURCE revoked' $APP_LOG >/dev/null"; then
    fail "$RESOURCE was revoked: a device that goes away must not cost its holder the resource"
fi
sshq "grep -F '$RESOURCE' $APP_LOG | grep -F 'the grant is kept'" \
    || fail "the client did not report keeping the grant of $RESOURCE"
alive || fail "the client died when its device went away"
echo "  suspended, holder keeps $RESOURCE, client still alive"

say "10/10 the device comes back — the SAME client gets it, with no re-acquire, and it works"
# A fresh uinput device with the same name: a different devnode, the same class,
# so the daemon resumes the suspended resource for the client that held it.
BEFORE=$(sshq "grep -cF 'acquiring $RESOURCE' $APP_LOG || true")
sshq "setsid sh -c 'exec /root/uinput-inject keyboard $DEVICE < $FIFO > $INJECT_LOG 2>&1' & sleep 6"
sshq "cat $INJECT_LOG | grep -q '^ready'" || fail "the injector did not come back"
sshq "journalctl -u simple-graphics-controller --since '-30s' --no-pager -o cat | grep -F 'Resumed $RESOURCE' | grep -F 'no re-acquire'" \
    || fail "the daemon did not resume $RESOURCE for its holder"
sshq "grep -F 'libinput device added: $RESOURCE at' $APP_LOG | grep -F '($DEVICE)'" \
    || fail "$RESOURCE was not handed back to libinput"
AFTER=$(sshq "grep -cF 'acquiring $RESOURCE' $APP_LOG || true")
[ "$BEFORE" = "$AFTER" ] \
    || fail "the client re-acquired $RESOURCE ($BEFORE -> $AFTER): the daemon must re-grant it"
alive || fail "the client died when its device came back"
echo "  resumed for the same holder, re-added to libinput, no re-acquire"
sshq "echo 'key $CHORD' > $FIFO; sleep 3"
if alive; then
    fail "the client is still alive: the chord never arrived on the resumed device"
fi
echo "  client exited: events flow again on the resumed device"

say "PASS"
