#!/bin/bash
# End-to-end proof that INPUT keeps working across a revoke/re-grant, with no
# human at the board.
#
# What it drives: a virtual keyboard created by uinput-inject while the daemon
# is RUNNING (the daemon adopts a device node that appears under it — no restart),
# an input-capable @sgc client on the screen, and sgc-steal preempting that
# keyboard (FairQueue) to force the revoke/re-grant cycle.
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
UNIT="${UNIT:-adguard-dashboard-slint}"
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
    sshq 'pkill -f "/root/uinput-injec[t]"; pkill -f "sleep 360[0]"; true' || true
    sshq "systemctl start $UNIT; true" || true
}
trap cleanup EXIT

say "preflight: uinput + harness binaries on $BOARD"
sshq 'ls /dev/uinput >/dev/null && [ -x /root/uinput-inject ] && [ -x /root/sgc-steal ]' \
    || fail "/dev/uinput (modprobe uinput) or /root/{uinput-inject,sgc-steal} missing"
if sshq 'pgrep -f "^/root/uinput-injec[t]" >/dev/null'; then
    fail "an injector is already running"
fi

say "1/7 stop $UNIT, start the virtual $DEVICE"
sshq "systemctl stop $UNIT"
sshq "rm -f $FIFO && mkfifo $FIFO"
# A holder keeps the FIFO writable, so the injector never sees EOF when one ssh
# command finishes writing to it.
sshq "setsid sh -c 'exec 3<>$FIFO; sleep 3600' >/dev/null 2>&1 & sleep 1
setsid sh -c 'exec /root/uinput-inject keyboard $DEVICE < $FIFO > $INJECT_LOG 2>&1' &"
sleep 2
sshq "grep -q '^ready' $INJECT_LOG" || fail "the injector did not report ready"
sshq "cat $INJECT_LOG"

say "2/7 the RUNNING daemon adopts the virtual device (hot-plug, no restart)"
ADOPTED=$(sshq "sleep 5; journalctl -u simple-graphics-controller --since '-30s' --no-pager -o cat | grep '$DEVICE' | grep 'plugged in while running'" || true)
[ -n "$ADOPTED" ] || fail "the daemon never adopted $DEVICE at runtime"
NODE=$(printf '%s' "$ADOPTED" | sed -n 's|.*Opened \([^ ]*\) .*|\1|p')
[ -n "$NODE" ] || fail "could not tell which node $DEVICE got: $ADOPTED"
echo "  $DEVICE is $NODE"

say "3/7 start the client and find the resource index of the virtual keyboard"
sshq "rm -f $APP_LOG; cd /root; setsid sh -c 'SLINT_DRM_MODE=3 exec $APP > $APP_LOG 2>&1' &"
sleep 7
LINE=$(sshq "grep 'libinput device added.*($DEVICE)' $APP_LOG | tail -1" || true)
INDEX=$(printf '%s' "$LINE" | sed -n 's/.*Input(Keyboard(\([0-9]*\))).*/\1/p')
[ -n "$INDEX" ] || fail "the client never registered $DEVICE (log: $LINE)"
RESOURCE="Input(Keyboard($INDEX))"
STEAL="keyboard:$INDEX"
echo "  $DEVICE is $RESOURCE"

say "4/7 negative control: tap A — the client must survive"
sshq "echo 'tap A' > $FIFO; sleep 2"
alive || fail "the client died on an ordinary key"

say "5/7 steal $RESOURCE for 2s — revoke, then re-grant"
sshq "/root/sgc-steal $STEAL 2000 1 500; sleep 2"
sshq "grep -F '$RESOURCE revoked' $APP_LOG | grep -F 'device removed from libinput'" \
    || fail "no revoke was logged for $RESOURCE"
sshq "grep -F 'libinput device added: $RESOURCE at' $APP_LOG | grep -F '($DEVICE)'" \
    || fail "$RESOURCE was not re-added to libinput after the re-grant"
alive || fail "the client died across the revoke/re-grant"
echo "  revoked, re-granted, re-added — client still alive"

say "6/7 inject the chord AFTER the resume — the client must exit"
sshq "echo 'key $CHORD' > $FIFO; sleep 3"
if alive; then
    fail "the client is still alive: the chord never arrived after the resume"
fi
echo "  client exited: input works again after the revoke/re-grant"

say "7/7 the injector dies — the daemon must let the device go"
sshq "pkill -f '/root/uinput-injec[t]'; sleep 6"
sshq "journalctl -u simple-graphics-controller --since '-30s' --no-pager -o cat | grep -F 'is gone; withdrawing' | grep -F '$NODE'" \
    || fail "the daemon kept $NODE after the process holding it died"

say "PASS"
