#!/bin/sh
# Steady-state memory of one dashboard, for comparing the LVGL and Slint apps.
#
#   measure-mem.sh <binary> <label>
#
# Runs the binary alone (killing any other dashboard first), then samples it at
# ~30s and ~60s: VmRSS/VmHWM/Threads from /proc/pid/status, the PSS breakdown
# from smaps_rollup (Pss_Anon = heap/stacks, Pss_File = mapped files, Pss_Shmem
# = shared memory incl. dma-buf/shmem mappings), the open DRM nodes and the
# scanout framebuffers registered with the DRM device.
#
# To attribute GPU usage: a Slint femtovg build has libEGL/libgallium/libGLdispatch
# mapped, an LVGL GBM build only libgbm (it uses gbm for buffers and renders on
# the CPU). See adguard-dashboard-slint/docs/handoff.md for measured numbers.
set -eu
BIN=$1
LABEL=$2

pkill -f '^/root/adguard-slint$' 2>/dev/null || true
pkill -f '^/root/adguard-dashboard' 2>/dev/null || true
sleep 2
setsid -f "$BIN" > "/tmp/mem-$LABEL.log" 2>&1 < /dev/null

sample() {
    PID=$(pgrep -f "^$BIN$" | head -1 || true)
    if [ -z "$PID" ]; then
        echo "### $LABEL: not running - last lines of /tmp/mem-$LABEL.log:"
        tail -4 "/tmp/mem-$LABEL.log"
        return 1
    fi
    echo "### $LABEL pid=$PID at ${1}s"
    grep -E "^(VmRSS|VmHWM|VmSize|Threads)" "/proc/$PID/status"
    grep -E "^(Pss|Pss_Anon|Pss_File|Pss_Shmem|Shared_Clean|Shared_Dirty|Private_Clean|Private_Dirty|Swap):" "/proc/$PID/smaps_rollup"
    echo "fds=$(ls /proc/$PID/fd | wc -l) dri=$(ls -l /proc/$PID/fd 2>/dev/null | grep -oE 'dri/(card[0-9]|renderD[0-9]+)' | sort -u | tr '\n' ' ')"
    echo "uptime=$(ps -o etime= -p "$PID" | tr -d ' ') cpu=$(ps -o pcpu= -p "$PID" | tr -d ' ')%"
}

sleep 30
sample 30 || { pkill -f "^$BIN$" 2>/dev/null || true; exit 0; }
echo "-- scanout framebuffers --"
head -8 /sys/kernel/debug/dri/0/framebuffer 2>/dev/null || true
sleep 30
sample 60 || true
pkill -f "^$BIN$" 2>/dev/null || true
sleep 2
