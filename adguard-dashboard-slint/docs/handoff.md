# adguard-dashboard-slint — handoff

Status: built, cross-compiled for the board, deployed and running on the GPU.
Everything below is measured, not assumed; the one open measurement is called out.

## What it is

`sgc-demos/adguard-dashboard-slint` — the AdGuard Home statistics dashboard as a
Slint UI, rendered through the `simple-graphics-controller` daemon on a DRM
lease. Same screen, same `/etc/agh-dash/config.toml` and same API rules as the
LVGL app in `../adguard-dashboard`; the two are alternatives (one DRM lease).

    src/main.rs   wiring: window, font registration, worker thread + UI timer
    src/agh.rs    /control/stats + /control/status client (ureq, serde_json)
    src/config.rs toml config, AGH_DASH_PASSWORD override, mode warning
    src/view.rs   snapshot -> Slint models, palette (mirrors the LVGL one)
    src/selfcheck.rs  offscreen render + per-colour pixel counts
    ui/main.slint header, protection badge, 4 chart cards, 3 table rows, footer
      (the page layout only; components live one per file: theme.slint,
       types.slint, card.slint, section-title.slint, bar-chart.slint,
       chart-card.slint, table-card.slint, protection-badge.slint)
    scripts/start-dashboard-slint.sh   board launcher
    Justfile      host build; board builds (GPU / CPU flavors)

Renderer: `femtovg` (OpenGL over gbm/EGL, i.e. the Mali GPU) with the software
renderer compiled in as well — the backend is `try_femtovg_then_software`, so GL
is preferred and CPU is the fallback. `--no-default-features --features software`
builds the CPU-only flavor. Input is opt-in via `--features input` (dynamic gnu
builds only, board needs libinput10 + libxkbcommon0): the dashboard stays
read-only, but the backend draws the mouse cursor with the CPU renderer and
Ctrl+Alt+Backspace quits. Off by default — a client must not hold a device it
cannot consume.

## Verified today

* Cross build (x86_64 → aarch64): `just build-gnu-aarch64`, 13,180,632 bytes,
  NEEDED `libgbm.so.1` + libc/libm/libgcc_s (EGL/GLES are dlopened by glutin).
* Offscreen `--self-check` **on the board**: data 874 queries / 96 blocked,
  24 buckets per series, then 2,476,800 pixels painted —
  `page=342611 card=2066842 grid=23100 badge=5107`, and bars on all four charts
  (2561 / 2825 / 1158 / 1023 px). Font registered from DejaVuSans.ttf.
* Live run: lease granted + acknowledged, plane[40] `allocated by = adguard-slint`,
  refreshing every 5 s, ~8–12% CPU.
* **GPU is really in use**: the process holds `dri/card0` *and* `dri/renderD128`
  (the render node — the CPU flavor only ever touches `card0`).

## Board state right now

    /root/simple-graphics-controller-gnu   daemon, started by /root/start-sgc.sh
    /root/adguard-slint                    the GPU dashboard, owns the screen
    /root/agh-slint.log, /root/sgc-daemon.log

    ssh root@10.21.50.53 '/root/start-sgc.sh'        # idempotent, safe to re-run
    ssh root@10.21.50.53 '/root/start-agh-slint.sh'  # stops the LVGL one, starts this
    ssh root@10.21.50.53 '/root/start-aghdash.sh'    # the LVGL dashboard instead

Watch it: `ssh root@10.21.50.53 'tail -f /tmp/agh-slint.log'`.

## Open items

1. **Steal behaviour of the GPU build — measure it.** The documented limitation
   (GL contexts live on the lease fd, so a revoke is fatal) comes from the earlier
   `slint-lease-client` femtovg test. In the one run we got today the app did
   *not* die: after the LVGL dashboard stole the lease, `adguard-slint` was still
   alive, still fetching, while the thief owned the plane — parked, or wedged,
   nobody checked. Re-measure before relying on either story.
2. **The daemon died twice today** after its clients were killed, with no obvious
   cause (once after a board reboot, once after the two dashboards were killed).
   Both times the next client failed with "connection refused". Worth a look in
   the daemon repo — `sgc` clients are sgc-or-die, so a silent daemon exit takes
   the screen down.
3. **Restart-on-death for the dashboard**: done. The app ships as a
   `Restart=always` unit, which is what carries the GL flavor through a steal —
   it cannot rebuild its renderer in-process, while the CPU flavor re-acquires in
   place.
4. **GPU + preemption survival in the fork** (optional, the real fix if wanted):
   teach `i-slint-backend-linuxsgc` to tear down and rebuild the renderer on
   re-grant, after which the GPU flavor would survive a steal too. Contained to
   the backend.
5. **Commit** — done and published: the library and ABI work is on crates.io
   (`libsgc-rs 0.2.0`, `libsgc-c 0.1.1`) and every repo is pushed, tags included.
6. The board reboots on its own (power/network); every reboot costs a daemon
   restart. `start-sgc.sh` now handles that in one command.

## Gotchas learned (all now encoded in scripts/docs)

* `setsid` **without `-f` execs** when the script is not a process-group leader,
  so a launcher blocks for the app's whole lifetime and never starts it. Use
  `setsid -f`. Both dashboards' launchers and the daemon launcher are fixed.
* Install the binary as `/root/adguard-slint`, not `adguard-dashboard-slint-gnu`:
  the kernel comm truncates at 15 chars, so both dashboards would show up in
  `/sys/kernel/debug/dri/0/state` as the same `adguard-dashboa`.
* The board has **no fontconfig and no freetype**: register `DejaVuSans.ttf` into
  fontique at startup or text has no font source.
* GL builds must **not** set `PKG_CONFIG_ALL_STATIC=1` (they link the board's
  libgbm/EGL/GLES dynamically); the CPU flavor does, for the font libs.
* The AdGuard series arrays cover the whole statistics interval, **oldest
  first** — chart the newest 24 (`newest()` in `src/agh.rs`), and never the head,
  which is the "charts don't show" bug from the LVGL dashboard.
* Table rows: a layout child **stretches to fill its parent** by default, which
  made every row 52px with the text pinned at the top; and a layout's
  `alignment` does **not** move text vertically - the Text's own
  `vertical-alignment` does, and it defaults to top. Fix: explicit row height
  (34px), cells with `vertical-alignment: center`. Measured with the self-check's
  `column ... text bands` / `separators` probe (row band 594-629, ink 605-617).
* Slint resolves `self` to the *current element*: an enclosing component's
  property is referenced by its bare name; `padding` only applies to layouts.
* The three top lists arrive as **single-key objects** (`{"10.21.50.1": 758}`),
  not `{domain, count}`; with `#[serde(default)]` on every field a mismatch
  degrades to empty labels and 0 counts instead of failing. Now parsed in both
  shapes, and the self-check bails on all-empty labels.
* Verify with `--self-check` (pixels), never by asking whether it "looks right".

## Memory comparison: Slint/GPU vs LVGL/GBM

Same screen, same data source, same 5 s refresh, both with GBM scanout buffers —
the LVGL app built with `-DLV_BUILD_CONF_PATH=<conf with
LV_USE_LINUX_DRM_GBM_BUFFERS 1>`, the Slint app with its default GPU renderer.
Measured on the board with `scripts/measure-mem.sh` (samples at 30 s and 60 s,
both stable, both single instances):

    metric                     Slint (GPU)      LVGL (GBM)
    VmRSS                      84.9 MB          70.0 MB
    Pss                        83.1 MB          65.9 MB
    Pss_Anon (heap/stacks)     25.1 MB          12.1 MB
    Pss_File (mapped files)    59.4 MB          55.0 MB
    Threads                      3                2
    idle CPU                     5.3%            14.2-15.6%
    GL libs mapped             libEGL, libGLdispatch,   libgbm only
                               libgallium, libgbm

The GPU build costs ~17 MB of PSS (+26%) and returns ~2.7× lower CPU. The extra
footprint is the GL machinery: +13 MB anonymous (EGL/GL heaps and buffers) and
+4 MB of mapped files (libEGL, libgallium).

Caveats worth knowing before quoting these: they are the app processes only — the
daemon (which holds the DRM master) and kernel-side dma-buf accounting are not
included, and this kernel's `/sys/kernel/debug/dri/0/clients` exposes no
per-client DRM memory. Each scanout buffer is 1720×1440 XR24 = 9.9 MB and its
mapping lands in Private_Clean for both. The *deployed* LVGL dashboard is the
non-GBM (dumb buffer) flavor; that footprint is not measured here.

Attribution trick: an LVGL GBM build has only `libgbm` mapped (gbm for buffers,
CPU rendering), while the Slint femtovg build has EGL/gallium/GLdispatch — so
mapped libraries, not `renderD128` alone, tell you who is really on the GPU.

## Uncommitted

`git -C ~/Projects/sgc/sgc-demos status`:

* new: `adguard-dashboard-slint/` (everything above), `scripts/measure-mem.sh`
* modified: `README.md`, `AGENT.md`,
  `adguard-dashboard/scripts/start-dashboard.sh` (setsid -f fix),
  `adguard-dashboard/CMakeLists.txt` (conf-path override for variant builds +
  GBM flavor support: it now detects `LV_USE_LINUX_DRM_GBM_BUFFERS` in the conf
  and links libgbm, which is how the GBM memory measurement was built)

Earlier work in the same repo is already committed: `6d28c1d` (LVGL dashboard),
`b8a3641` (AGENT.md).
