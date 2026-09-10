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
rust_target=$(sed -n 's/^rust-std-target=//p' "$HYPER_SYSROOT/share/hyper/manifest")
case "$rust_target" in aarch64-unknown-hyper|riscv64-unknown-hyper) ;; *)
    echo "check-rust-std.sh: invalid SDK Rust target" >&2; exit 2 ;;
esac
mkdir -p "$2"
std_output=$(CDPATH='' cd -- "$2" && pwd)

manifest=$3
"$HYPER_SYSROOT/bin/hyper-cargo" fetch --manifest-path "$manifest" --locked
for std_link_mode in dynamic static; do
    export CARGO_TARGET_DIR="$std_output/cargo/$std_link_mode"
    HYPER_RUST_STD=1 HYPER_LINK_MODE="$std_link_mode" \
        "$HYPER_SYSROOT/bin/hyper-cargo" build \
        --manifest-path "$manifest" --release --locked --offline
    source_binary=$CARGO_TARGET_DIR/$rust_target/release/hyper-std-smoke
    if ! cmp -s "$source_binary" "$std_output/std-$std_link_mode"; then
        install -m 0755 "$source_binary" "$std_output/std-$std_link_mode"
    fi
    "$HYPER_SYSROOT/bin/hyper-brand-elf" "--check-$std_link_mode" "$std_output/std-$std_link_mode"
done
CARGO_TARGET_DIR="$std_output/cargo/check" HYPER_RUST_STD=1 "$HYPER_SYSROOT/bin/hyper-cargo" clippy \
    --manifest-path "$manifest" --release --locked --offline -- -D warnings
