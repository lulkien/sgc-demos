#!/bin/sh
# Start (or restart) the AdGuard dashboard on a board, detached, logging to
# /tmp/agh-dash.log. Copy it to the board and run it there:
#
#     scp scripts/start-dashboard.sh root@<board>:/root/start-aghdash.sh
#     ssh root@<board> '/root/start-aghdash.sh'
#
# The @sgc daemon must be running first: the app takes a DRM lease from it and
# has no session to render into otherwise.
set -eu
install -d -m 700 /etc/agh-dash
pkill -f '^/root/adguard-dashboard-gnu$' 2>/dev/null || true
sleep 1
setsid /root/adguard-dashboard-gnu > /tmp/agh-dash.log 2>&1 < /dev/null
sleep 2

pid=$(pgrep -f '^/root/adguard-dashboard-gnu$' | head -1)
echo "adguard-dashboard pid ${pid:-none}; log /tmp/agh-dash.log"
[ -n "$pid" ] || { echo "did not start - is the @sgc daemon running?" >&2; exit 1; }
