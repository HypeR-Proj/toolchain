#!/bin/sh
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0
set -eu
root=$(CDPATH='' cd -- "$(dirname "$0")/../../.." && pwd)
cd "$root"
# Validate SDK assembly, exported interfaces and consumers against the installed
# SDK. ABI, libc, Rust libraries and applications own their source unit tests.
for arch in ${SDK_CI_ARCH:-aarch64 riscv64}; do
    case "$arch" in aarch64|riscv64) ;; *) echo "unsupported SDK architecture: $arch" >&2; exit 2 ;; esac
    make sdk-check app-check app-sdk-test ARCH="$arch"
done
