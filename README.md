# sgc demos

Standalone projects for simple-graphics-controller (@sgc) — no workspace: every
directory is its own project (own Cargo.toml, its own Meson or CMake project).

Dependencies arrive as git refs, git submodules or vendored artifacts; nothing
builds against a sibling checkout. libsgc-rs and the slint fork (branch
`sgc-lease-1.17`) come in as Cargo git deps, cJSON and tomlc99 are git
submodules, libsgc is a vendored archive, and lvgl comes from the fork by git
ref. A Slint app enables only the `backend-linuxsgc` feature and never names the
backend crate or SgcClient.

- sgc-drm-client — Rust: acquire a DRM card lease from @sgc, raw-ioctl modeset +
  paint loop on the granted fd
- sgc-fbdev-client — Rust: acquire the fbdev resource from @sgc, draw via linfb
  (input + animation)
- slint-lease-client — Rust: a Slint UI on a DRM lease via the linuxsgc backend
  (slint fork git dep, feature backend-linuxsgc; add --features input for
  keyboard/mouse/touch from the granted devices — gnu builds only, board needs
  libinput10)
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
