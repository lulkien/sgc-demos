# sgc demos

Standalone projects for simple-graphics-controller (@sgc) — no workspace: every
directory is its own project (own Cargo.toml, its own Meson or CMake project).

Dependencies arrive as published crates, git refs, git submodules or vendored
artifacts; nothing builds against a sibling checkout. libsgc-rs comes from
crates.io, the slint fork (branch `dev/1.18-sgc`) as a Cargo git dep, cJSON and
tomlc99 are git submodules, libsgc is a vendored archive, and lvgl comes from the
fork by git ref. A Slint app enables only the `backend-linuxsgc` feature and never names the
backend crate or SgcClient.

- adguard-dashboard — C + LVGL (CMake): AdGuard Home's statistics page rendered
  through @sgc (LV_USE_SGC, no input); vendored libsgc, submoduled cJSON/tomlc99,
  committed lv_conf.h, `--self-check` pixel read-back
- adguard-dashboard-slint — Rust + Slint: the same dashboard as a Slint UI, same
  layout and same config file, software renderer, no input feature, offscreen
  `--self-check`
- ir-remote-test — Rust + Slint: an IR remote test screen that shows every key
  the seat delivers, plus an optional raw-evdev read-back (`--raw`) for the keys
  the xkb mapping drops; ships the remote's keymap (`keymap/x98h.toml`), the unit
  that loads it (`packaging/ir-keymap.service`) and the procedure
  (`docs/ir-keymap.md`)

The two dashboards read `/etc/agh-dash/config.toml` and are alternatives: there is
one DRM lease, so only one of them runs at a time. `ir-remote-test` is a test
tool — it needs the seat too, so it runs instead of a dashboard, not beside it.

Build each from its own directory (`just` for the current host,
`just build-gnu-aarch64` for the board; the two dashboards document their own
recipes) — see the per-project comments for cross/board builds.
