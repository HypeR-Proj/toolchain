#!/bin/sh
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0
set -eu
repository=$(CDPATH='' cd -- "$(dirname "$0")/../../.." && pwd)
temporary=$(mktemp -d "${TMPDIR:-/tmp}/hyper-loader-arch.XXXXXX")
trap 'rm -rf "$temporary"' EXIT
# Apple's ASan runtime is not usable in every macOS host environment; Linux CI
# runs both sanitizers. UBSan remains enabled on all hosts.
case "$(uname -s)" in
    Linux) sanitizers=address,undefined ;;
    *) sanitizers=undefined ;;
esac
for architecture in aarch64 riscv64; do
    case "$architecture" in
        aarch64) flags='-D__aarch64__' ;;
        riscv64) flags='-D__riscv -D__riscv_xlen=64' ;;
    esac
    "${HOST_CC:-clang}" -std=c17 -Wall -Wextra -Werror \
        -U__aarch64__ -U__riscv -U__riscv_xlen $flags \
        "$repository/lib/dynamic-loader/tests/architecture.c" -o "$temporary/probe"
    "$temporary/probe"
    case "$architecture" in
        riscv64) runtime_flags='-DTEST_RISCV' ;;
        aarch64) runtime_flags='' ;;
    esac
    "${HOST_CC:-clang}" -std=c17 -Wall -Wextra -Werror \
        -fsanitize="$sanitizers" -fno-omit-frame-pointer $runtime_flags \
        -idirafter "$repository/lib/hyper/include" -idirafter "$repository/sdk/abi/include" \
        "$repository/lib/dynamic-loader/tests/runtime.c" -o "$temporary/runtime"
    "$temporary/runtime"
done
echo 'verified loader architecture relocation formulas and machine flags'
