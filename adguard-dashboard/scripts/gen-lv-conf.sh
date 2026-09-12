#!/usr/bin/env bash
#
# gen-lv-conf.sh - generate lv_conf.h from <lvgl>/lv_conf_template.h and apply
# this project's configuration with sed, so the build needs no
# -DCONFIG_LV_* / lv_conf overrides on the cmake command line.
#
#   ./gen-lv-conf.sh [--gbm] [--sgc] [lvgl dir] [output lv_conf.h]
#
# --gbm               configure LV_USE_LINUX_DRM_GBM_BUFFERS=1 (GBM/DMA-BUF
#                     buffers instead of dumb buffers); default output is then
#                     lv_conf_gbm.h instead of lv_conf.h.
# --sgc               configure LV_USE_SGC=1 (take the DRM lease and the input
#                     devices from the @sgc daemon); default output is then
#                     lv_conf_sgc.h.
# --no-input          with --sgc: do not acquire the input devices (for apps
#                     that take no input - a device a client cannot consume
#                     must stay available to the others).
# --gbm --sgc         both: GBM buffers on a lease taken from the daemon, written
#                     to lv_conf_sgc_gbm.h.
#
# Defaults: lvgl dir = <script dir>/lvgl (here a symlink to the fork checkout),
# output = <script dir>/lv_conf.h. The output deliberately lives in the consumer
# project, not inside the lvgl checkout: every consumer of a shared lvgl tree
# then owns its own configuration, and nothing untracked appears in the fork.
#
# Re-run after every lvgl update, and after editing the SETTINGS section below.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

GBM=0
SGC=0
NO_INPUT=0
positional=()
for arg in "$@"; do
    case "$arg" in
        --gbm) GBM=1 ;;
        --sgc) SGC=1 ;;
        --no-input) NO_INPUT=1 ;;
        -h|--help) sed -n '3,27p' "${BASH_SOURCE[0]}"; exit 0 ;;
        -*) echo "error: unknown option '$arg'" >&2; exit 2 ;;
        *) positional+=("$arg") ;;
    esac
done

LVGL_DIR="${positional[0]:-$SCRIPT_DIR/lvgl}"
if [ "$SGC" -eq 1 ] && [ "$GBM" -eq 1 ]; then
    CONF="${positional[1]:-$SCRIPT_DIR/lv_conf_sgc_gbm.h}"
elif [ "$SGC" -eq 1 ]; then
    CONF="${positional[1]:-$SCRIPT_DIR/lv_conf_sgc.h}"
elif [ "$GBM" -eq 1 ]; then
    CONF="${positional[1]:-$SCRIPT_DIR/lv_conf_gbm.h}"
else
    CONF="${positional[1]:-$SCRIPT_DIR/lv_conf.h}"
fi
TPL="$LVGL_DIR/lv_conf_template.h"

[ -f "$TPL" ] || { echo "error: template not found: $TPL" >&2; exit 1; }

missing=0

# Rewrite one "#define <name> <value>" line, keeping indentation and an optional
# trailing "/**< ... */" comment.
set_conf() {
    local name="$1" value="$2"

    if ! grep -qE "^[[:space:]]*#define[[:space:]]+$name[[:space:]]" "$CONF"; then
        printf '  !! %s not found in the template\n' "$name" >&2
        missing=$((missing + 1))
        return
    fi

    sed -i -E \
        -e "s|^([[:space:]]*#define[[:space:]]+$name[[:space:]]+).*([[:space:]]+/\*\*<.*\*/)[[:space:]]*$|\1$value\2|" \
        -e 't' \
        -e "s|^([[:space:]]*#define[[:space:]]+$name[[:space:]]+).*$|\1$value|" \
        "$CONF"
}

cp -f "$TPL" "$CONF"

# The template body is wrapped in "#if 0 /* Set this to "1" to enable content */".
sed -i 's|^#if 0 /\* Set this to|#if 1 /* Set this to|' "$CONF"

# ---------------------------------------------------------------------------
# SETTINGS
# ---------------------------------------------------------------------------

# Display / DRM driver
set_conf LV_COLOR_DEPTH                    32                 # XRGB8888 dumb buffers
set_conf LV_DEF_REFR_PERIOD                16
set_conf LV_USE_LINUX_DRM                  1
set_conf LV_USE_LINUX_DRM_GBM_BUFFERS      "$GBM"

# @sgc daemon driver (--sgc): DRM lease from the daemon, and (unless
# --no-input) its input devices for an app that can consume them.
SGC_INPUT=$SGC
[ "$NO_INPUT" -eq 1 ] && SGC_INPUT=0
set_conf LV_USE_SGC                        "$SGC"
set_conf LV_SGC_INPUT                      "$SGC_INPUT"
set_conf LV_USE_EVDEV                      "$SGC_INPUT"   # only a client that feeds input needs it

# Logging + telemetry (driver log and fps/cpu lines go to stdout)
set_conf LV_USE_LOG                        1
set_conf LV_LOG_LEVEL                      LV_LOG_LEVEL_INFO
set_conf LV_LOG_PRINTF                     1
set_conf LV_USE_SYSMON                     1
set_conf LV_USE_PERF_MONITOR               1
set_conf LV_USE_PERF_MONITOR_LOG_MODE      1

# Heap (widgets widget tree of the demos needs more than the template default)
set_conf LV_MEM_SIZE                       "(8 * 1024U * 1024U)"

# Demos: benchmark for fps numbers, widgets because the benchmark uses its scene
set_conf LV_BUILD_DEMOS                    1
set_conf LV_USE_DEMO_WIDGETS               1
set_conf LV_USE_DEMO_BENCHMARK             1

# Fonts the widgets/benchmark demos reference
for size in 12 14 16 18 20 22 24 26 28; do
    set_conf "LV_FONT_MONTSERRAT_$size" 1
done

# ---------------------------------------------------------------------------
# Verify
# ---------------------------------------------------------------------------
variant=dumb
if [ "$SGC" -eq 1 ] && [ "$GBM" -eq 1 ]; then
    variant=sgc+gbm
elif [ "$SGC" -eq 1 ]; then
    variant=sgc
    [ "$NO_INPUT" -eq 1 ] && variant=sgc-noinput
elif [ "$GBM" -eq 1 ]; then
    variant=gbm
fi
echo "generated $CONF from $(basename "$TPL") (variant: $variant)"
grep -nE '^#if 1 /\* Set this to|Set this to "1" to enable content' "$CONF" | head -2
for name in LV_COLOR_DEPTH LV_DEF_REFR_PERIOD LV_USE_LINUX_DRM LV_USE_LINUX_DRM_GBM_BUFFERS \
            LV_USE_SGC \
            LV_USE_LOG LV_LOG_LEVEL LV_LOG_PRINTF LV_USE_SYSMON LV_USE_PERF_MONITOR \
            LV_USE_PERF_MONITOR_LOG_MODE LV_MEM_SIZE LV_BUILD_DEMOS LV_USE_DEMO_WIDGETS \
            LV_USE_DEMO_BENCHMARK LV_FONT_MONTSERRAT_12 LV_FONT_MONTSERRAT_14 \
            LV_FONT_MONTSERRAT_16 LV_FONT_MONTSERRAT_18 LV_FONT_MONTSERRAT_20 \
            LV_FONT_MONTSERRAT_22 LV_FONT_MONTSERRAT_24 LV_FONT_MONTSERRAT_26 \
            LV_FONT_MONTSERRAT_28; do
    grep -E "^[[:space:]]*#define[[:space:]]+$name[[:space:]]" "$CONF"
done

if [ "$missing" -ne 0 ]; then
    echo "error: $missing setting(s) missing - template layout changed?" >&2
    exit 1
fi
