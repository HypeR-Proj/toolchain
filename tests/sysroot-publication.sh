#!/bin/sh
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

set -eu
if [ "$#" -ne 3 ]; then
    echo "usage: sysroot-publication.sh ABI_SOURCE LIB_SOURCE RUST_SOURCE" >&2
    exit 2
fi
repository=$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)
temporary=$(mktemp -d "${TMPDIR:-/tmp}/hyper-sysroot-test.XXXXXX")
trap 'rm -rf "$temporary"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
output=$temporary/sysroot
mkdir -p "$output/include"
printf 'old header\n' > "$output/include/obsolete.h"
mkdir -p "$output/share/hyper/rust"
printf 'old Rust SDK file\n' > "$output/share/hyper/rust/obsolete.rs"
# Fail at the final host-tool build, after staging installation has finished.
if HOST_CC=false sh "$repository/scripts/build-sysroot.sh" "$1" "$2" "$3" "$output" > "$temporary/failure.log" 2>&1; then
    echo "expected host compiler failure" >&2
    exit 1
fi
grep -q 'Installing:.*libhyper.a' "$temporary/failure.log"
test "$(cat "$output/include/obsolete.h")" = 'old header'
test "$(cat "$output/share/hyper/rust/obsolete.rs")" = 'old Rust SDK file'
test ! -e "$output/lib"
test ! -e "$output.publish-lock"
if ! sh "$repository/scripts/build-sysroot.sh" "$1" "$2" "$3" "$output" > "$temporary/success.log" 2>&1; then
    cat "$temporary/success.log" >&2
    exit 1
fi
test ! -e "$output/include/obsolete.h"
test ! -e "$output/share/hyper/rust/obsolete.rs"
test -f "$output/include/hyper/native.h"
test -f "$output/lib/libhyper.a"
test -x "$output/bin/hyper-brand-elf"
test -x "$output/bin/hyper-cargo"
test -f "$output/share/hyper/rust/hyper-os/Cargo.toml"
test ! -e "$output.publish-lock"
# Fail the final rename after the old sysroot has moved to the backup.
mkdir "$temporary/bin"
cat > "$temporary/bin/mv" <<'MOVE'
#!/bin/sh
case "$1" in
    */.hyper-sysroot.*/sysroot) exit 1 ;;
esac
exec /bin/mv "$@"
MOVE
chmod +x "$temporary/bin/mv"
printf 'preserved\n' > "$output/rollback-marker"
if PATH="$temporary/bin:$PATH" sh "$repository/scripts/build-sysroot.sh" "$1" "$2" "$3" "$output" > "$temporary/rename.log" 2>&1; then
    echo "expected publication rename failure" >&2
    exit 1
fi
test "$(cat "$output/rollback-marker")" = preserved
test -f "$output/lib/libhyper.a"
test ! -e "$output.publish-lock"
# A second publisher must leave the completed output untouched.
mkdir "$output.publish-lock"
if sh "$repository/scripts/build-sysroot.sh" "$1" "$2" "$3" "$output" > "$temporary/locked.log" 2>&1; then
    echo "concurrent publisher was not rejected" >&2
    exit 1
fi
test -f "$output/lib/libhyper.a"
rmdir "$output.publish-lock"
echo "verified sysroot replacement, rollback, and publisher exclusion"
