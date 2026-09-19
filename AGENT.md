# AGENT.md — sgc-demos

## Purpose
Standalone demo clients — and one real application — for the simple-graphics-controller stack. Every directory is its own project (no workspace span): the Rust demos each have their own Cargo.toml and git-dep the slint fork branch `dev/1.18-sgc`, `c-samples` is a Meson project, `adguard-dashboard` is a CMake project using the lvgl fork. Demos demonstrate client-side resource ownership patterns, event loops, and protocol interaction with the `@sgc` daemon.

Nothing here builds against a sibling checkout: dependencies arrive as git refs, git submodules or vendored artifacts (see "C Projects" below).

## Demo Portfolio
| Demo | Backend | Features | Purpose |
|---|---|---|---|
| `slint-lease-client` | DRM + slint UI | default: `software`; optional: `femtovg`, `input` | Runs a Slint UI on a DRM lease granted by the daemon; demonstrates full app integration with revoke/suspend/resume; UI marker moves to ui/main.slint |
| `c-samples` | DRM (Meson) | `sgc-drm-c`, `sgc-drm-cpp` | C and C++ clients against the libsgc C ABI; takes the archive and headers through `-Dsgc_dir=` (predates the vendoring rule below) |
| `adguard-dashboard` | DRM + LVGL (CMake) | `LV_USE_SGC`, no input | A real application: AdGuard Home's statistics page in LVGL, rendered through the daemon via the lvgl fork. Vendored libsgc, submoduled cJSON/tomlc99, committed lv_conf.h, `--self-check` pixel read-back |
| `adguard-dashboard-slint` | DRM + Slint (Cargo) | `backend-linuxsgc`, `femtovg` (GPU) + `software` fallback, `input` opt-in | The same dashboard as a Slint UI: same layout, same `/etc/agh-dash/config.toml`, worker thread for the API, offscreen `--self-check` that counts pixels; `--features input` adds the mouse cursor and Ctrl+Alt+Backspace |
| `input-harness` | test tooling (no client) | `uinput-inject`, `sgc-steal` | Virtual keyboard/mouse/touchscreen via `/dev/uinput` plus a resource steal probe, driving the revoke/re-grant cycle; `scripts/input-resume-test.sh` (8 steps) asserts the running daemon adopts the virtual device (no restart), that a device created under a running CLIENT reaches it (the daemon pushes its resource list), that input delivery survives a re-grant (negative control included), and that the daemon withdraws the device once its injector dies |

## Shared Resource Ownership Model (all demos)
- **Client holds the canonical fd** — `SgcClient::acquire(resource)` stores the granted fd in `held: HashMap<Resource, OwnedFd>`; client owns it
- **`fd()` lends a DUP** — `SgcClient::fd(resource)` returns a fresh dup; the borrower owns the dup and must drop it; the canonical stays with the client
- **Revoke drops canonical** — when `SgcEvent::Revoked { resource }` arrives, the client drops the canonical fd; the app's borrower learns via the event and drops its dup
- **Re-grant stores new canonical** — after revoke, a new `Grant` creates a fresh lease fd; `fd()` lends a new dup
- **One held fd per resource** — the client can hold multiple resources simultaneously (e.g. Fbdev + Drm + Input), each with its own canonical fd and lent dups

## The Dashboard Projects (C/LVGL and Rust/Slint)
`adguard-dashboard` (C, CMake) and `adguard-dashboard-slint` (Rust, Cargo) render
the same screen, from the same `/etc/agh-dash/config.toml`, and must not build
against a sibling checkout:

- **No path dependencies.** Published crates (libsgc-rs, from crates.io) and git refs are both fine — never `../`. lvgl comes from the fork by git ref (`LVGL_REPO` / `LVGL_REF` in `adguard-dashboard/CMakeLists.txt`). libsgc is either the vendored prebuilt archive (`adguard-dashboard/third_party/libsgc`, whose `REF` records the libsgc-c commit and whose README records provenance), or fetched and built by ref (`-DSGC_REF=`, needs cargo), or an archive built elsewhere (`-DSGC_ARCHIVE=`). `c-samples` predates this rule and still takes its archive + headers through `-Dsgc_dir=`.
- **Third-party code is submodules or vendored headers, not copies.** cJSON and tomlc99 are git submodules under `adguard-dashboard/third_party/`; curl's public headers are vendored because upstream curl is a large repo, and they are only needed on hosts without `libcurl4-gnutls-dev`.
- **`lv_conf.h` is committed** (the `--sgc --no-input` variant), so no build step generates configuration. `scripts/gen-lv-conf.sh` regenerates it only when the lvgl ref moves; `scripts/vendor-libsgc.sh` refreshes the vendored archive.
- **Cross build** (x86_64 host → aarch64 board): `cmake -B build-arm64 -DCMAKE_TOOLCHAIN_FILE=toolchain-aarch64.cmake`, then `cmake --build build-arm64 -j`. The toolchain file points pkg-config at the arm64 multiarch `.pc` files.
- **Board deploy**: `scripts/start-dashboard.sh` (C) and `scripts/start-dashboard-slint.sh` (Rust) kill any previous instance, start detached, log to `/tmp/agh-dash.log` / `/tmp/agh-slint.log` and fail loudly if the @sgc daemon is not running. The daemon must be up first — an @sgc client is sgc-or-die. Only one dashboard runs at a time: there is a single DRM lease, and each launcher stops the other.
- **Slint specifics.** The Rust dashboard renders on the **GPU** by default: the crate enables the fork's `backend-linuxsgc` feature with the `femtovg` renderer (OpenGL over gbm/EGL), and the backend itself is `try_femtovg_then_software` — GL first, CPU renderer as the fallback when GL init fails. A lease revoke is survivable in both flavors: on a re-grant the GL stack is dropped and rebuilt from the fresh lease fd (same pid, the plane stays ours) and the CPU flavor re-inits its dumb-buffer display. Keep `Restart=always` in the unit for what is left — a rebuild that fails on the mode the new lease carries, a daemon restart (the client's socket dies with it), or a config error. `--no-default-features --features software` builds the CPU flavor. Both renderers stay compiled in so the offscreen `--self-check` can use the software path while the window uses the GPU. Input is **opt-in** (`--features input`, dynamic gnu builds only; the board needs `libinput10` + `libxkbcommon0`): the UI stays read-only, but the backend puts the granted devices to work — the CPU renderer composites the mouse cursor and Ctrl+Alt+Backspace quits the event loop. A featureless build holds no device it cannot consume. Logging is `log` + `env_logger` on `RUST_LOG` (default `info`): the per-refresh progress line is `debug`, so a unit that sets `RUST_LOG=info` keeps only state changes and failures in the journal, and `RUST_LOG=debug` brings one line per refresh back. The board image has neither fontconfig nor freetype, so the app registers `DejaVuSans.ttf` into fontique at startup (what `slint-lease-client` does too); Slint is event-driven on one thread, so the fetch loop lives on a worker thread and the UI thread only drains a channel.
- **Verify by reading pixels, not by looking at the screen.** Both dashboards have a `--self-check` that renders and counts pixels: the C one lets a couple of refreshes land, then reads the display's active buffer back (`lv_display_get_buf_active`) and reports per-widget geometry and pixel counts with a table's text pixels as a control; the Slint one renders offscreen with the software renderer (no window, no lease) and counts pixels per colour. That is how "the charts don't show" turned out to be a data-slice bug rather than a rendering one.

## Rust Best Practices (per rust-skills, Rust demos)
- [`own-borrow-over-clone`] — `fd()` returns a DUP; the canonical stays owned by the client; borrowers must drop their dup. See `acquire_display_fd` / `acquire` patterns
- [`own-arc-shared`] — Use `Arc<T>` for thread-safe shared ownership only when needed; most demo data stays on one thread (client thread drives the event loop)
- [`own-refcell-interior`] — Use `RefCell<T>` for interior mutability in single-threaded code; not currently in demos but the pattern is established in the codebase
- [`own-cow-conditional`] — Use `Cow<'a, T>` for conditional ownership where appropriate
- [`err-result-over-panic`] — Return `Result<T, E>` instead of panicking for recoverable errors; all demo `main()` functions return `Result<(), Box<dyn std::error::Error>>` or similar
- [`err-from-impl`] — `SgcError` implements `From<ProtocolError>` and `From<std::io::Error>` via `#[from]` to enable `?` operator; demos use `?` throughout
- [`err-question-mark`] — Use `?` operator for clean error propagation; demos never call `.unwrap()` in production paths
- [`err-no-unwrap-prod`] — Avoid `unwrap()` in production code; use `?`, `expect()`, or handle errors; demos use `expect()` only for invariants indicating bugs (e.g. font file must exist)
- [`expect-bugs-only`] — Use `expect()` only for invariants that indicate bugs, not user errors or runtime conditions; e.g. font file read failure is a bug, not a recoverable error
- [`mem-with-capacity`] — Use `Vec::with_capacity()` when size is known; demos use `Vec::with_capacity` for event payloads
- [`perf-iter-over-index`] — Prefer iterators over manual indexing; demos use `.iter()`, `.find_map()`, `.collect()` over manual indexing
- [`num-nonzero`] — Use `NonZero*` types to forbid zero and unlock niche optimization; not yet in demos but the principle applies to resource indices
- [`api-from-not-into`] — Implement `From<T>`, not `Into<U>` — `SgcError::from` gives you `Into` for free
- [`api-must-use`] — Mark types and functions with `#[must_use]` when ignoring results is likely a bug; demos: `acquire` return value, `fd()` result, `start_event_loop` dispatch
- [`doc-all-public`] — Document all public items with `///` doc comments; all public types and functions in all crates have doc comments
- [`doc-errors-section`] — Include `# Errors` section documenting all error variants; `SgcError` has `# Errors` doc section
- [`doc-panics-section`] — Include `# Panics` section for functions that can panic under documented conditions; demos use `expect()` with doc comments
- [`doc-question-mark`] — Use `?` in examples, not `.unwrap()`; examples should demonstrate proper error handling
- [`obs-tracing-over-log`] — Use `tracing` for structured, span-aware diagnostics; demos use `tracing_subscriber::EnvFilter` + `tracing::fmt()` in the controller, individual demos use `eprintln!` for simplicity (acceptable for demo code)
- [`obs-structured-fields`] — Record structured key-value fields, not values interpolated into the message string
- [`anti-lock-across-await`] — Never hold `Mutex`/`RwLock` across `.await`; demos are synchronous, no async
- [`anti-clone-excessive`] — Don't clone when borrowing works; demos: `client.fd(&resource)` clones the `OwnedFd` (intended, the borrower owns the dup), but `held` entries are borrowed via `&Resource` where possible
- [`anti-type-erasure`] — Don't use `Box<dyn Trait>` when `impl Trait` works; demo types use concrete `SgcClient`, `Resource`, `OwnedFd`
- [`anti-stringly-typed`] — Don't use strings where enums or newtypes would provide type safety; all demos use `Resource` enum, `InputResource` enum, not stringly-typed

## Key Patterns Across Demos
### Resource Acquisition
```rust
// Common pattern: acquire + lend dup
let fd = client.acquire(resource.clone())?;       // client now owns canonical
let borrow_fd = client.fd(&resource)?;            // borrower gets a dup; client keeps canonical
// borrower uses borrow_fd; when done, drops it
// revoke drops canonical + tells borrower to stop
```

### Event Loop
```rust
// Common pattern: start_event_loop dispatches events
client.start_event_loop(|event| match event {
    SgcEvent::Revoked { resource } => {
        // drop canonical, stop borrower tasks
    }
    SgcEvent::Granted { resource, fd } => {
        // new canonical; lend fresh dup to borrower
        let borrow_fd = client.fd(&resource)?;
    }
});
```

### Multi-Resource Holding
```rust
// Client can hold multiple resources simultaneously
client.acquire(Resource::Fbdev)?;
client.acquire(Resource::Drm { card: 0 })?;
client.acquire(Resource::Input(InputResource::Mouse(0)))?;
// Each held resource has its own canonical fd
// fd() lends a dup per resource
```

## Build & Run Conventions
- Each demo is its own Cargo project; build with `cargo build --release` from its directory
- `slint-lease-client`: default features = `["software"]`; add `--features femtovg` for GPU; `--features input` for keyboard/mouse/touch (gnu only, not musl)
- Run as **root** (opens `/dev/dri` + `/dev/input`): `RUST_LOG=info ./target/release/<demo>`
- `slint-lease-client` env vars: `SLINT_BACKEND=linuxsgc` (or leave empty; it's the only backend enabled)
- C projects build with CMake (`adguard-dashboard`) or Meson (`c-samples`); cross builds use `toolchain-aarch64.cmake` plus the `aarch64-linux-gnu-*` host toolchain
- `adguard-dashboard` on a board: copy the binary next to the other clients and run `scripts/start-dashboard.sh`; `--config <path>` overrides `/etc/agh-dash/config.toml`, and the AdGuard credentials live only in that file (root:root 0600) — never in git
- `adguard-dashboard --self-check` runs the pixel read-back once data has arrived, prints it, and exits

## Test Conventions
- Each demo may have integration tests in `#[cfg(test)] mod integration_tests { }` (simple-graphics-controller pattern)
- Use `sendfd::SendWithFd`/`RecvWithFd` for SCM_RIGHTS fd passing where needed
- Fake servers for protocol testing are in `libsgc-rs` itself (`fake_server`, `FakeController`)
- Demos test the full end-to-end flow: connect → acquire → pump events → revoke/re-grant

## Common Pitfalls to Avoid (all demos)
- ❌ Do NOT call `.unwrap()` in production paths — all demos use `?` or `expect()` with documented conditions
- ❌ Do NOT forget to drop the borrower's dup — the borrower owns it; when the borrower's scope ends, it drops, which is intentional (the dup is borrowed, not the canonical)
- ❌ Do NOT ignore `SgcEvent::Revoked` — the render/input tasks must stop when the resource is revoked; the event carries the resource so the app can match it
- ❌ Do NOT hold locks across await points — demos are synchronous; if migrating to async, use `spawn_blocking` for CPU-intensive work
- ❌ Do NOT accept `&Vec<T>` when `&[T]` works — resource types use enums directly, not Vec-wrapped
- ❌ Do NOT clone the canonical fd unnecessarily — `fd()` explicitly returns a DUP; the client keeps the canonical
- ❌ Do NOT mix `Arc` and `Rc` carelessly — if data never leaves the event-loop thread, `Rc` is preferred (avoids atomic refcount ops)
- ❌ Do NOT forget to `Ack` a grant — the server waits up to 5s for the client's Ack; missing Ack only logs, never gates the queue
- ❌ Do NOT use `.unwrap()` on font file read in `slint-lease-client` — the font file existence is a build-time invariant; use `expect()` with a doc comment if the path might change
- ❌ Do NOT combine `input` feature with musl builds — musl static cannot link system libinput; `input` is gnu-only
- ❌ Do NOT make a build call into a sibling project — no generator scripts, no `../../some-checkout`. Dependencies come from git refs, git submodules or the vendored `third_party/libsgc`; a genuine path input is an explicit `-D...=<path>` option
- ❌ Do NOT generate `lv_conf.h` as part of a build — it is committed; regenerating it is a deliberate step taken when the lvgl ref moves
- ❌ Do NOT request a resource the app cannot consume — the LVGL dashboard sets `LV_SGC_INPUT 0` and the Slint one keeps `input` out of its default features for the same reason; holding what you cannot use takes it from other clients. When a UI does need input, enable it deliberately (the Slint backend then draws the cursor and quits on Ctrl+Alt+Backspace)
- ❌ Do NOT run `apt` on a board unattended — package-manager load has rebooted one mid-install and left dpkg interrupted