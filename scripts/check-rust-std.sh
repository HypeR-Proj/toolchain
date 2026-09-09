#!/bin/sh
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

set -eu
if [ "$#" -ne 3 ]; then
    echo "usage: check-rust-std.sh SDK OUTPUT MANIFEST" >&2
    exit 2
fi
export HYPER_SYSROOT
HYPER_SYSROOT=$(CDPATH='' cd -- "$1" && pwd)
mkdir -p "$2"
std_output=$(CDPATH='' cd -- "$2" && pwd)
export CARGO_TARGET_DIR="$std_output/cargo"
manifest=$3
"$HYPER_SYSROOT/bin/hyper-cargo" fetch --manifest-path "$manifest" --locked
for std_link_mode in dynamic static; do
    HYPER_RUST_STD=1 HYPER_LINK_MODE="$std_link_mode" \
        "$HYPER_SYSROOT/bin/hyper-cargo" build \
        --manifest-path "$manifest" --release --locked --offline
    install -m 0755 "$CARGO_TARGET_DIR/aarch64-unknown-hyper/release/hyper-std-smoke" \
        "$std_output/std-$std_link_mode"
    "$HYPER_SYSROOT/bin/hyper-brand-elf" "--check-$std_link_mode" "$std_output/std-$std_link_mode"
done
HYPER_RUST_STD=1 "$HYPER_SYSROOT/bin/hyper-cargo" clippy \
    --manifest-path "$manifest" --release --locked --offline -- -D warnings
