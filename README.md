# sgc demos

Standalone demo projects for simple-graphics-controller (@sgc). No workspace —
every directory is its own project: each Rust crate is standalone with its own
Cargo.toml/Cargo.lock, and c-samples is one Meson project.

All dependencies are git refs: libsgc-rs for the client demos, and the slint
fork (branch `sgc-lease-1.17`) for slint-lease-client. The slint demo only uses
the `backend-linuxsgc` slint feature and never names the backend crate or
SgcClient.

- sgc-drm-client — Rust: acquire a DRM card lease from @sgc, raw-ioctl
  modeset + paint loop on the granted fd
- sgc-fbdev-client — Rust: acquire the fbdev resource from @sgc, draw via
  linfb (input + animation)
- slint-lease-client — Rust: a Slint UI on a DRM lease via the linuxsgc
  backend (slint fork git dep, feature backend-linuxsgc)
- c-samples — C/C++ (Meson): sgc-drm-c / sgc-drm-cpp, link libsgc.a from the
  libsgc-c repo (-Dsgc_dir=...)

Build each from its own directory with its Justfile (`just` for the current
host, `just build-musl-aarch64` / `just build-gnu-aarch64` for the board;
c-samples: `just` and `just board`) — see the per-project comments for
cross/board builds.
