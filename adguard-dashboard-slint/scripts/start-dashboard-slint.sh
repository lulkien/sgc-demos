#!/bin/sh
# Start (or restart) the Slint AdGuard dashboard on a board, detached, logging to
# /tmp/agh-slint.log. Copy it to the board and run it there:
#
#     scp scripts/start-dashboard-slint.sh root@<board>:/root/start-agh-slint.sh
#     ssh root@<board> '/root/start-agh-slint.sh'
#
# The binary is installed as /root/adguard-slint: a comm name of at most 15
# characters, so /sys/kernel/debug/dri/0/state shows `allocated by = adguard-slint`
# and not the truncated `adguard-dashboa` that both dashboards would otherwise
# share.
#
# The @sgc daemon must be running first: the app takes a DRM lease from it and
# has no session to render into otherwise. Only one client can hold that lease,
# so the LVGL dashboard (adguard-dashboard, binary adguard-dashboard-gnu) is
# stopped here too - the two are alternatives, not companions.
set -eu
install -d -m 700 /etc/agh-dash
pkill -f '^/root/adguard-slint$' 2>/dev/null || true
pkill -f '^/root/adguard-dashboard-gnu$' 2>/dev/null || true
sleep 1
setsid -f /root/adguard-slint > /tmp/agh-slint.log 2>&1 < /dev/null || true
sleep 2

pid=$(pgrep -f '^/root/adguard-slint$' | head -1)
echo "adguard-dashboard-slint pid ${pid:-none}; log /tmp/agh-slint.log"
[ -n "$pid" ] || { echo "did not start - is the @sgc daemon running?" >&2; exit 1; }
