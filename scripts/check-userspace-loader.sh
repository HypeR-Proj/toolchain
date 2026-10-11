#!/bin/sh
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0
set -eu
repository=$(CDPATH='' cd -- "$(dirname "$0")/../../.." && pwd)
temporary=$(mktemp -d "${TMPDIR:-/tmp}/hyper-userspace-loader.XXXXXX")
trap 'rm -rf "$temporary"' EXIT
case "$(uname -s)" in
    Linux) sanitizers=address,undefined ;;
    *) sanitizers=undefined ;;
esac
for architecture in aarch64 riscv64; do
    case "$architecture" in riscv64) flags=-DTEST_RISCV ;; *) flags= ;; esac
    for mode in static dynamic; do
        case "$mode" in dynamic) mode_flags=-DTEST_DYNAMIC_LOADER ;; *) mode_flags= ;; esac
        "${HOST_CC:-clang}" -std=c17 -Wall -Wextra -Werror $flags $mode_flags \
            -fsanitize="$sanitizers" -fno-omit-frame-pointer \
            -I "$repository/lib/elf-loader" \
            -idirafter "$repository/lib/hyper/include" -idirafter "$repository/sdk/abi/include" \
            "$repository/bootstrap/stage1/userspace-loader/tests/startup.c" -o "$temporary/startup"
        "$temporary/startup"
    done
done
