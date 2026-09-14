# libsgc (vendored)

    include/libsgc.h, include/sgc.hpp   the C ABI headers
    lib/aarch64/libsgc.a                static library for the boards
    REF                                 the libsgc-c commit this was built from

The default build links these, so building the dashboard needs neither cargo nor
the network. `libsgc-c` declares its dependencies by version (`libsgc-rs = "0.2"`
from crates.io), so a from-source build is reproducible as well: the archive is
about not needing cargo or the network, not about pinning.

This artifact is BEHIND what the source builds: `REF` predates
`SGC_EVENT_ADVERTISED`, the C ABI's event for a changed resource list. Re-vendor
(`scripts/vendor-libsgc.sh` after bumping `SGC_REF`) before relying on the event
or on the library matching a source build.

Provenance: built from `https://github.com/lulkien/libsgc-c.git` at the commit in
`REF`, for `aarch64-unknown-linux-gnu`, `cargo build --release`, then
`aarch64-linux-gnu-strip --strip-debug` (23 MB -> 15 MB; the symbol table the
linker needs is kept, debug info is dropped).

Refresh after bumping `SGC_REF` in `CMakeLists.txt`:

    scripts/vendor-libsgc.sh            # or scripts/vendor-libsgc.sh <ref>

Other architectures are not vendored: the CMake file falls back to fetching
libsgc-c by git ref and building it with cargo.
