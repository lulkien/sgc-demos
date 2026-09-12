#!/bin/sh
# Refresh the vendored libsgc used by the default build: a prebuilt aarch64
# archive plus headers under third_party/libsgc, so building the dashboard
# needs neither cargo nor the network. Run this after bumping SGC_REF in
# CMakeLists.txt (or pass a ref explicitly):
#
#     scripts/vendor-libsgc.sh [<ref>]
#
# The archive is stripped: it keeps the symbol table the linker needs and drops
# debug info (23 MB -> 15 MB).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
ref=${1:-$(sed -n 's/^set(SGC_REF[[:space:]]*"\([^"]*\)".*/\1/p' "$root/CMakeLists.txt" | head -1)}
[ -n "$ref" ] || { echo "no ref given and no SGC_REF in CMakeLists.txt" >&2; exit 2; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "fetching libsgc-c $ref"
git clone -q https://github.com/lulkien/libsgc-c.git "$tmp/libsgc-c"
git -C "$tmp/libsgc-c" checkout -q "$ref"

echo "building the aarch64 archive"
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    cargo build --release --target aarch64-unknown-linux-gnu \
        --manifest-path "$tmp/libsgc-c/Cargo.toml"

out="$root/third_party/libsgc"
mkdir -p "$out/include" "$out/lib/aarch64"
cp "$tmp/libsgc-c/include/libsgc.h" "$tmp/libsgc-c/include/sgc.hpp" "$out/include/"
cp "$tmp/libsgc-c/target/aarch64-unknown-linux-gnu/release/libsgc.a" "$out/lib/aarch64/libsgc.a"
aarch64-linux-gnu-strip --strip-debug "$out/lib/aarch64/libsgc.a"
printf '%s\n' "$ref" > "$out/REF"

ls -l "$out/lib/aarch64/libsgc.a"
echo "vendored libsgc $ref into third_party/libsgc"
