# adguard-dashboard

AdGuard Home statistics on an LVGL display, rendered through a **@sgc DRM
lease**. The app polls AdGuard Home's own API and mirrors its statistics page:
four charts over two rows of tables. Read-only — the display takes no input
device, so nothing scrolls or reacts to a pointer.

    header   AdGuard Home statistics                    <updated Ns ago>
    badge    [ Protection: enabled ]   <- boxed, green when on / red when off
    row 1    4 charts:  DNS queries | Blocked by filters |
                        Blocked malware / phishing | Blocked adult websites
    row 2    General statistics          |  Top clients
    row 3    Top queried domains         |  Top blocked domains
    footer   AdGuard Home v0.107.79                  <endpoint>

Section titles are 20 px over 16 px content, with extra air under the title.
Table columns are derived from the card's measured width - the numbers column
takes ~26% (capped at 110-260 px) and the label column the rest - so the grid
spans the card at any resolution instead of a hardcoded pixel width.

## Data

`GET /control/stats` (HTTP basic auth) supplies everything, with the four charts
taken straight from the per-bucket arrays:

| Chart / row | API field |
| --- | --- |
| DNS queries | `num_dns_queries`, series `dns_queries[]` |
| Blocked by filters | `num_blocked_filtering`, series `blocked_filtering[]` |
| Blocked malware / phishing | `num_replaced_safebrowsing`, series `replaced_safebrowsing[]` |
| Blocked adult websites | `num_replaced_parental`, series `replaced_parental[]` |
| General statistics | the counters above, `num_replaced_safesearch`, `avg_processing_time` |
| Top clients / queried domains / blocked domains | `top_clients[]`, `top_queried_domains[]`, `top_blocked_domains[]` |

`GET /control/status` adds `protection_enabled` (the badge under the header) and
`version` (the footer).

The series arrays cover the server's whole statistics interval, **oldest first**
(a 7-day interval arrives as 168 hourly buckets). The dashboard charts the newest
24 of them: charting the head would plot the oldest, mostly idle hours and make
the charts look empty. The interval length itself is not shown - it is a server
setting, not a statistic.

Note: `num_replaced_safesearch` has no time series in the API, so it appears only
as a row in the general statistics table.

## Configuration

    install -m 600 config.example.toml /etc/agh-dash/config.toml     # root-owned

`--config <path>` overrides the default `/etc/agh-dash/config.toml`. The file
holds the AdGuard credentials, so the app warns when it is group/world-readable
and never logs the password. `AGH_DASH_PASSWORD` overrides the password from the
file — convenient for trying it out without storing a secret anywhere.

## Dependencies

* libcurl — either the dev package (`libcurl4-gnutls-dev` natively,
  `libcurl4-gnutls-dev:arm64` for cross builds), or, when a host has no dev
  package, the vendored headers in `third_party/curl/include` plus
  `-DCURL_LIBRARY=<path to libcurl.so>` (e.g. the device's own
  `/usr/lib/aarch64-linux-gnu/libcurl.so.4`, copied off the board into
  `~/.local/share/agh-deps/aarch64/` — the linker records its soname, so the
  binary still runs there unchanged; add `-Wl,--allow-shlib-undefined`, which the
  CMake file does for you)
* LVGL from the fork, by git ref (`-DLVGL_REPO` / `-DLVGL_REF`, or
  `-DLVGL_ARCHIVE_URL`)
* libsgc - the vendored archive and headers in `third_party/libsgc` (see its
  README for provenance). Other architectures fetch and build `libsgc-c` by git
  ref (`-DSGC_REF`); `-DSGC_ARCHIVE=<libsgc.a>` links one built elsewhere, e.g.
  from the libsgc-dev package
* git submodules: cJSON 1.7.18 (MIT), tomlc99 (MIT)
* vendored in-tree: curl 8.14.1 public headers (curl licence), libsgc.a

## Build

    git submodule update --init --recursive          # cJSON + tomlc99
    cmake -B build-arm64 -DCMAKE_TOOLCHAIN_FILE=toolchain-aarch64.cmake
    cmake --build build-arm64 -j

That is the whole build. `lv_conf.h` is committed here, lvgl is fetched from the
fork by git ref, and libsgc is the vendored archive in `third_party/libsgc` - so
no sibling checkout, no cargo and no config-generation step is involved. Override
points: `-DCURL_LIBRARY=<libcurl.so>` on a host without libcurl's dev package,
`-DSGC_ARCHIVE=<libsgc.a>` for an archive built elsewhere, `-DSGC_REF=<ref>` to
build libsgc from source instead of using the vendored archive. The config is
regenerated with `scripts/gen-lv-conf.sh --sgc --no-input <lvgl checkout>
lv_conf.h`, the archive with `scripts/vendor-libsgc.sh`.

The committed `lv_conf.h` is the `--sgc --no-input` variant, and `--no-input`
matters: the dashboard takes no input device, and a client that cannot consume a
resource must not hold it (the daemon keeps the mouse and keyboard available to
other clients).

## Run on the board

    scp build-arm64/adguard_dashboard root@10.21.50.53:/root/adguard-dashboard-gnu
    ssh root@10.21.50.53 'install -d -m 700 /etc/agh-dash'
    scp config.toml root@10.21.50.53:/etc/agh-dash/config.toml   # chmod 600 there
    scp scripts/start-dashboard.sh root@10.21.50.53:/root/start-aghdash.sh

    # daemon first (the app is sgc-or-die), then the dashboard
    ssh root@10.21.50.53 '/root/start-aghdash.sh'      # scripts/start-dashboard.sh

AdGuard Home listens on `127.0.0.1:8080` on that board, so with
`base_url = "http://127.0.0.1:8080"` the credentials never leave the machine.

Debug aid: `--self-check` lets two refreshes land, then reads the display's
active buffer back and prints, per chart, its on-screen size, how many pixels
carry the series colour and how many carry the empty-state hint (plus a table's
text pixels as a control). That answers "is it drawn?" without looking at the
screen - it is how the chart-slice bug above was found:

    [selfcheck] chart 0: 389x366 at (28,127) points=24 bars_px=2620 hint_px=0
    [selfcheck] chart 2: 389x366 at (878,127) points=24 bars_px=0 hint_px=141

Expected log lines: `[agh-dash] http://127.0.0.1:8080 as 'admin', refresh every 5s`,
`[agh-dash] display 1720x1440`, then `[agh-dash] refreshed: N queries, M blocked`
every refresh. A rejected credential shows as
`[agh-dash] refresh failed (credentials rejected)` and as a red header line.
