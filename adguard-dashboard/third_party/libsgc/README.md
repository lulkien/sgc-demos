# libsgc (vendored)

    include/libsgc.h, include/sgc.hpp   the C ABI headers
    lib/aarch64/libsgc.a                static library for the boards
    REF                                 the libsgc-c commit this was built from

The default build links these, so building the dashboard needs neither cargo nor
the network. Linking one fixed archive also pins `libsgc-rs` - `libsgc-c` does
not track its `Cargo.lock`, so a from-source build resolves that dependency at
whatever its repository's current HEAD is.

Provenance: built from `https://github.com/lulkien/libsgc-c.git` at the commit in
`REF`, for `aarch64-unknown-linux-gnu`, `cargo build --release`, then
`aarch64-linux-gnu-strip --strip-debug` (23 MB -> 15 MB; the symbol table the
linker needs is kept, debug info is dropped).

Refresh after bumping `SGC_REF` in `CMakeLists.txt`:

    scripts/vendor-libsgc.sh            # or scripts/vendor-libsgc.sh <ref>

Other architectures are not vendored: the CMake file falls back to fetching
libsgc-c by git ref and building it with cargo.
