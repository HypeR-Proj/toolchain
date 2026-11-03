#!/bin/sh
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0
set -eu
root=$(CDPATH='' cd -- "$(dirname "$0")/../../.." && pwd)
cd "$root"
python3 sdk/toolchain/tests/test_cargo_fetch.py
python3 sdk/toolchain/tests/test_native_library.py
python3 sdk/toolchain/tests/test_cmake_incremental.py
python3 sdk/toolchain/tests/test_publication_ownership.py
# Validate SDK assembly, exported interfaces and consumers against the installed
# SDK. ABI, libc, Rust libraries and applications own their source unit tests.
# The owning module invocation has already selected and leased this architecture
# view. Preparing another architecture here would wait on our own read lock.
case "${ARCH:-aarch64}" in
    aarch64|riscv64) ;;
    *) echo "unsupported SDK architecture: $ARCH" >&2; exit 2 ;;
esac
make sdk-check app-check app-sdk-test
