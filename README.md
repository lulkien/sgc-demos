# sgc demos

Standalone projects for simple-graphics-controller (@sgc) — no workspace: every
directory is its own project (own Cargo.toml, its own Meson or CMake project).

Dependencies arrive as published crates, git refs, git submodules or vendored
artifacts; nothing builds against a sibling checkout. libsgc-rs comes from
crates.io, the slint fork (branch `dev/1.18-sgc`) as a Cargo git dep, cJSON and
tomlc99 are git submodules, libsgc is a vendored archive, and lvgl comes from the
fork by git ref. A Slint app enables only the `backend-linuxsgc` feature and never names the
backend crate or SgcClient.

- c-samples — C/C++ (Meson): sgc-drm-c / sgc-drm-cpp, link libsgc.a from the
  libsgc-c repo (-Dsgc_dir=...)
- adguard-dashboard — C + LVGL (CMake): AdGuard Home's statistics page rendered
  through @sgc (LV_USE_SGC, no input); vendored libsgc, submoduled cJSON/tomlc99,
  committed lv_conf.h, `--self-check` pixel read-back
- adguard-dashboard-slint — Rust + Slint: the same dashboard as a Slint UI, same
  layout and same config file, software renderer, no input feature, offscreen
  `--self-check`

Both dashboards read `/etc/agh-dash/config.toml` and are alternatives: there is
one DRM lease, so only one of them runs at a time.

Build each from its own directory with its Justfile (`just` for the current
host, `just build-musl-aarch64` / `just build-gnu-aarch64` for the board;
c-samples: `just` and `just board`; the two dashboards: see their READMEs) — see
the per-project comments for cross/board builds.
