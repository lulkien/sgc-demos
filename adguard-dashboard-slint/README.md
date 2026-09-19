# adguard-dashboard-slint

The AdGuard Home statistics dashboard as a **Slint** UI, rendered through the
`simple-graphics-controller` daemon (`@sgc`) on a DRM lease — the Slint
counterpart of the LVGL app in `../adguard-dashboard`, with the same layout and
the same config file. The two are alternatives: only one at a time can hold the
lease.

    header   AdGuard Home statistics                    <updated Ns ago>
    badge    [ Protection: enabled ]   <- boxed, green when on / red when off
    row 1    4 charts:  DNS queries | Blocked by filters |
                        Blocked malware / phishing | Blocked adult websites
    row 2    General statistics          |  Top clients
    row 3    Top queried domains         |  Top blocked domains
    footer   AdGuard Home v0.107.79                  <endpoint>

## Layout

    src/main.rs         wiring: window, font registration, worker thread + UI timer
    src/agh.rs          /control/stats + /control/status client (ureq, serde_json)
    src/config.rs       toml config, AGH_DASH_PASSWORD override, mode warning
    src/view.rs         snapshot -> Slint models; palette mirrored from ui/theme.slint
    src/selfcheck.rs    offscreen render + per-colour pixel counts
    ui/main.slint       the page: it only arranges the components below
    ui/theme.slint      Theme global - palette, type scale, spacing
    ui/types.slint      ChartData / TableData / RowData, filled by src/view.rs
    ui/card.slint       the frame every section sits in
    ui/section-title.slint
    ui/bar-chart.slint  bars from stretch ratios; Slint has no chart widget
    ui/chart-card.slint title + running total + bars + empty-state hint
    ui/table-card.slint headed table, row hairlines, eliding labels
    ui/protection-badge.slint
    scripts/start-dashboard-slint.sh   board launcher
    Justfile            host build; board builds (GPU / CPU flavors)

## Fitting the panel

The page is laid out from the panel, not from a canvas of its own. There is no
`width`/`height` on the Window: the linuxsgc backend sizes it from the DRM
lease's mode, the window publishes that size into `Theme.panel-w`/`panel-h`, and
every length follows from it. Header, badge and footer take fixed line boxes, the
three card rows split the rest (charts 30%, one table row 35% each), and each
table card shows the rows that fit its height at `Theme.row-min` — a 1366x768
panel shows seven of the nine top-list rows, a 1440-tall panel shows all nine.
Type and spacing scale with `Theme.scale`: 1.0 down to a 1000px-tall panel, then
tapering to a floor of 0.8.

A **declared** Window size does not do any of that, it pins the layout. Measured
with 1720x1440 declared on a 1366x768 panel, the page was laid out at 1720x1440
and the panel showed its top-left corner alone: the fourth chart card cut in
half, the second table row and the footer off screen entirely. The card widths
come from `Theme.content-w` rather than from `parent.width` for the same reason
in reverse: reading a row's own layout info from inside one of its children
closes a Slint binding loop
(`root.layoutinfo-h -> width -> layoutinfo-h -> root.layoutinfo-h`).

## What the framework owns

Nothing in `src/main.rs` mentions the backend or `SgcClient`: the crate enables
the slint feature `backend-linuxsgc` and the backend selector installs it when
the first window is created. That backend connects to `@sgc`, acquires the card
lease, and survives a revoke by parking the display stack and resuming it after
the re-grant.

Two deliberate choices follow from that:

* **GPU rendering** — `femtovg` over gbm/EGL, the default feature. The board has
  a Mali GPU on panfrost, and the backend is `try_femtovg_then_software`: it
  prefers GL and falls back to the CPU renderer if GL init fails. The trade-off
  is that a lease revoke is fatal for GL: its EGL/GL context is created on the
  lease fd, so a steal ends the process. Run it under a unit with
  `Restart=always` if the screen must come back without help. Build
  `--no-default-features --features software` for the CPU flavor that parks and
  resumes instead. Both renderers stay compiled in, so the window uses the GPU
  while the offscreen `--self-check` still uses the software renderer.
* **input is opt-in** (`--features input`, dynamic gnu builds only). Nothing in
  the dashboard reacts to a click — it stays a read-only view — but with the
  devices granted the backend puts them to work: the software renderer
  composites the mouse cursor, and Ctrl+Alt+Backspace quits the event loop (the
  kiosk's escape hatch). It is off by default because a client that cannot
  consume a resource must not hold one: a featureless build leaves the mouse and
  keyboard to whoever can use them, the same reason the LVGL app sets
  `LV_SGC_INPUT 0`. The board needs `libinput10` + `libxkbcommon0`.

## Data

Careful with the three top lists (`top_clients`, `top_queried_domains`,
`top_blocked_domains`): AdGuard returns each entry as a **single-key object**,
`{"10.21.50.1": 758}` / `{"pass.proton.me": 23}`, not `{domain, count}` as its
schema suggests. `entries()` in `src/agh.rs` accepts both shapes and drops
unrecognised entries, and `--self-check` fails if a non-empty list parses to
all-empty labels - a silent all-defaults parse is how these three tables first
shipped blank with 0 counts.

Same endpoints and rules as the LVGL dashboard (`GET /control/stats` and
`GET /control/status`, HTTP basic auth, the newest 24 buckets of each series
array — see that project's README for why the *tail* is charted, and for the
field-to-card mapping). Fetching runs on a worker thread and the UI thread only
drains a channel on a 200 ms timer, so a slow request never blocks rendering,
and a failed refresh keeps the last snapshot on screen instead of blanking it.

## Configuration

The same file the LVGL dashboard reads:

    install -d -m 700 /etc/agh-dash
    install -m 600 config.example.toml /etc/agh-dash/config.toml

`--config <path>` overrides the default `/etc/agh-dash/config.toml`, and
`AGH_DASH_PASSWORD` overrides the password from the file. Keys the C app has and
this one does not (such as `verify_tls`) are ignored, so one file serves both.
This build is plain-HTTP only (no TLS stack, to keep the arm64 cross build
small), which suits an AdGuard Home on the loopback.

## Build

    just                            # release build for the current host
    just build-gnu-aarch64          # the board: GPU (femtovg), dynamic aarch64/gnu
    just build-gnu-aarch64-software # the board: CPU renderer (survives a revoke)
    just build-gnu-aarch64-input    # the board: GPU + input (cursor, Ctrl+Alt+Backspace)

The arm64 build needs `cargo`, the `aarch64-unknown-linux-gnu` target, and the
arm64 pkg-config files for freetype (fontique); the Justfile lists the flavors.
slint itself comes from
the fork as a git dependency (branch `dev/1.18-sgc`), so the first build
fetches and compiles it.

## Run on the board

    scp target/aarch64-unknown-linux-gnu/release/adguard-dashboard-slint \
        root@<board>:/root/adguard-slint
    scp scripts/start-dashboard-slint.sh root@<board>:/root/start-aghdash.sh
    ssh root@<board> '/root/start-aghdash.sh'

Installed as `/root/adguard-slint`: at most 15 characters, so the kernel comm
name (and with it `allocated by` in `/sys/kernel/debug/dri/0/state`) tells this
app apart from the LVGL dashboard, whose comm would otherwise truncate to the
same `adguard-dashboa`.

The board image ships neither fontconfig nor freetype, so the app registers
`DejaVuSans.ttf` into the text pipeline itself at startup; without that file,
text has no font source.

Logging follows `RUST_LOG` (`log` + `env_logger`, default `info`). The
per-refresh progress line is `debug`, so the journal carries only state changes
and failures by default; `RUST_LOG=debug` on the command line — or in a unit
drop-in — brings one line per refresh back.

## Self-check (verify by reading pixels, not by looking)

    adguard-dashboard-slint --self-check [--config <path>] [--size WxH,WxH...]

Renders the dashboard with the software renderer into a plain buffer — no
window, no lease, no daemon — once per panel size (default
`1366x768,1720x1440,1920x1080`, or `SLINT_SELFCHECK_SIZE`), and prints what it
painted, per colour:

    [selfcheck] data: 21515 queries, 3436 blocked, 24 buckets/series, version v0.107.79
    [selfcheck] 1366x768: rendered in 9 ms
    [selfcheck] 1366x768: of 1049088 pixels: page=... card=... grid=... badge=...
    [selfcheck] 1366x768: chart 0 bars px=3715
    [selfcheck] 1366x768: card row 0: 328x189 at (13,82) | 328x189 at (350,82) | ...
    [selfcheck] 1366x768: content x=13..1352 y=16..754 of 1366x768
    [selfcheck] 3 panel size(s) ok

Every size must pass, and a failure names the panel:

* the card grid is four charts then two rows of two tables — a missing row or a
  row with the wrong number of cards fails;
* the cards of one row have to come out equal (they get explicit weights; an
  odd remainder may differ by a pixel);
* the page has to stay inside the buffer (`content ... of WxH`) — this is the
  check that fails first when a fixed size or a row that is too tall creeps back
  in, and it is why the sizes are a parameter instead of a constant: a check
  that only ever renders the canvas the markup was designed on cannot see a
  panel smaller than that canvas;
* with data present, at least one bar pixel on the charts.

That is the Slint counterpart of the LVGL app's `--self-check`, and it is how the
layout and the data path get checked without a screen.
