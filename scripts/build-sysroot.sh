#!/bin/sh
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

set -eu

if [ "$#" -ne 3 ]; then
    echo "usage: build-sysroot.sh ABI_SOURCE LIB_SOURCE OUTPUT" >&2
    exit 2
fi

abi_source=$1
lib_source=$2
output=$3
script_directory=$(CDPATH='' cd -- "$(dirname "$0")" && pwd)
repository=$(CDPATH='' cd -- "$script_directory/.." && pwd)

if [ ! -f "$abi_source/include/hyper/native.h" ]; then
    echo "build-sysroot.sh: ABI source does not contain include/hyper/native.h" >&2
    exit 2
fi
if [ ! -f "$lib_source/CMakeLists.txt" ]; then
    echo "build-sysroot.sh: Lib source does not contain CMakeLists.txt" >&2
    exit 2
fi
if [ -z "$output" ] || [ "$output" = / ]; then
    echo "build-sysroot.sh: refusing unsafe output path" >&2
    exit 2
fi
# Keep the caller's relative destination valid while preventing the first
# dirname/basename invocation from interpreting a leading dash as an option.
case "$output" in
    -*) output=./$output ;;
esac

compiler=${CLANG:-clang}
host_compiler=${HOST_CC:-clang}
archiver=${LLVM_AR:-llvm-ar}
archive_indexer=${LLVM_RANLIB:-llvm-ranlib}
sdk_version=${HYPER_SDK_VERSION:-source}
source_revision=${HYPER_SDK_SOURCE_REVISION:-unknown}
for value in "$sdk_version" "$source_revision"; do
    case "$value" in
        ''|*[!A-Za-z0-9._+-]*)
            echo "build-sysroot.sh: invalid SDK identity: $value" >&2
            exit 2
            ;;
    esac
done
# Serialize publishers and keep all installation effects in a fresh sibling.
# The backup is restored if publication fails or receives a handled signal.
mkdir -p "$(dirname "$output")"
output_parent=$(CDPATH='' cd -- "$(dirname "$output")" && pwd)
output_name=$(basename "$output")
case "$output_name" in
    /|.|..|'') echo "build-sysroot.sh: unsafe output name" >&2; exit 2 ;;
esac
output=$output_parent/$output_name
publication_lock=$output.publish-lock
if ! mkdir "$publication_lock"; then
    echo "build-sysroot.sh: could not acquire publication lock $publication_lock" >&2
    exit 2
fi
transaction=
cleanup() {
    if [ -n "$transaction" ]; then
        if [ -e "$transaction/previous" ] || [ -L "$transaction/previous" ]; then
            if [ ! -e "$output" ] && [ ! -L "$output" ]; then
                mv "$transaction/previous" "$output" || return
            fi
        fi
        rm -rf "$transaction"
    fi
    rmdir "$publication_lock"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
transaction=$(mktemp -d "$output_parent/.hyper-sysroot.XXXXXX")
build_directory=$transaction/build
staged_output=$transaction/sysroot

cmake -S "$lib_source" -B "$build_directory" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_C_COMPILER="$compiler" \
    -DCMAKE_ASM_COMPILER="$compiler" \
    -DCMAKE_AR="$archiver" \
    -DCMAKE_RANLIB="$archive_indexer" \
    -DCMAKE_SYSTEM_NAME=Generic \
    -DCMAKE_SYSTEM_PROCESSOR=aarch64 \
    -DCMAKE_C_COMPILER_TARGET=aarch64-none-elf \
    -DCMAKE_ASM_COMPILER_TARGET=aarch64-none-elf \
    -DHYPER_ARCH=aarch64 \
    -DHYPER_ABI_INCLUDE_DIR="$abi_source/include"
cmake --build "$build_directory"
cmake --install "$build_directory" --prefix "$staged_output"

install -d "$staged_output/include/hyper" "$staged_output/bin"
install -m 0644 "$abi_source/include/hyper/native.h" "$staged_output/include/hyper/native.h"
install -m 0755 "$repository/bin/hyper-clang" "$staged_output/bin/hyper-clang"
install -d "$staged_output/lib/hyper/aarch64"
install -m 0644 "$repository/lib/aarch64/hyper-native.ld" \
    "$staged_output/lib/hyper/aarch64/hyper-native.ld"
abi_revision=$(sed -n \
    's/^#define HYPER_NATIVE_ABI_REVISION UINT64_C(\([0-9][0-9]*\))$/\1/p' \
    "$abi_source/include/hyper/native.h")
case "$abi_revision" in
    ''|*[!0-9]*)
        echo "build-sysroot.sh: invalid Native ABI revision" >&2
        exit 2
        ;;
esac
install -d "$staged_output/share/hyper"
{
    printf 'sdk-version=%s\n' "$sdk_version"
    printf 'source-revision=%s\n' "$source_revision"
    printf 'host=%s-%s\n' "$(uname -s)" "$(uname -m)"
    printf 'target=aarch64-none-elf\n'
    printf 'abi-revision=%s\n' "$abi_revision"
} > "$staged_output/share/hyper/manifest"
"$host_compiler" -std=c17 -Wall -Wextra -Werror \
    -I"$abi_source/include" "$repository/tools/brand-elf.c" \
    -o "$staged_output/bin/hyper-brand-elf"

# No existing sysroot is touched until every compiler and install succeeds.
if [ -e "$output" ] || [ -L "$output" ]; then
    mv "$output" "$transaction/previous"
fi
mv "$staged_output" "$output"
