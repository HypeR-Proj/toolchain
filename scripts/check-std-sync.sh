#!/bin/sh
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

set -eu
repository=$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)
temporary=$(mktemp -d "${TMPDIR:-/tmp}/hyper-std-sync.XXXXXX")
trap 'rm -rf "$temporary"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
for source in "$repository/../lib/src/thread.c" "$repository/tests/sync-host.c"; do
    "${HOST_CC:-clang}" -std=c17 -D_POSIX_C_SOURCE=200809L -DHYPER_THREAD_HOST_TEST \
        -I"$repository/../lib/include" -I"$repository/../abi/include" \
        -c "$source" -o "$temporary/$(basename "$source").o"
done
HYPER_RUST_LIBRARY="$(rustc --print sysroot)/lib/rustlib/src/rust/library" \
    HYPER_STD_OVERLAY="$repository/rust-std/overlay" RUSTC_BOOTSTRAP=1 \
    rustc --edition=2024 --test "$repository/tests/sync-host.rs" \
    -C "link-arg=$temporary/thread.c.o" -C "link-arg=$temporary/sync-host.c.o" \
    -o "$temporary/sync-host"
python3 - "$temporary/sync-host" <<'PY'
import subprocess
import sys
subprocess.run([sys.argv[1]], timeout=30, check=True)
PY
