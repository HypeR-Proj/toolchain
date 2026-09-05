# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

ABI_SOURCE ?=
LIB_SOURCE ?=
RUST_SOURCE ?=
OUTPUT ?= target/sysroot/aarch64
TEST_OUTPUT ?= target/tests/aarch64
SMOKE_SOURCE ?= tests/smoke.c
RUST_SMOKE_MANIFEST ?= tests/rust-smoke/Cargo.toml

.PHONY: verify-sources sysroot check test-publication clean

verify-sources:
	@test -n "$(ABI_SOURCE)" || { echo "ABI_SOURCE is required" >&2; exit 2; }
	@test -n "$(LIB_SOURCE)" || { echo "LIB_SOURCE is required" >&2; exit 2; }
	@test -n "$(RUST_SOURCE)" || { echo "RUST_SOURCE is required" >&2; exit 2; }

sysroot: verify-sources
	sh scripts/build-sysroot.sh "$(ABI_SOURCE)" "$(LIB_SOURCE)" "$(RUST_SOURCE)" "$(OUTPUT)"

check: sysroot test-publication
	mkdir -p "$(TEST_OUTPUT)"
	HYPER_SYSROOT="$(abspath $(OUTPUT))" bin/hyper-clang \
		-std=c17 -Wall -Wextra -Werror -c "$(SMOKE_SOURCE)" \
		-o "$(TEST_OUTPUT)/smoke.o"
	HYPER_SYSROOT="$(abspath $(OUTPUT))" bin/hyper-clang \
		"$(TEST_OUTPUT)/smoke.o" -o "$(TEST_OUTPUT)/smoke"
	"$(OUTPUT)/bin/hyper-brand-elf" --check "$(TEST_OUTPUT)/smoke"
	CARGO_TARGET_DIR="$(abspath $(TEST_OUTPUT))/rust" \
		HYPER_SYSROOT="$(abspath $(OUTPUT))" \
		"$(OUTPUT)/bin/hyper-cargo" build \
		--manifest-path "$(RUST_SMOKE_MANIFEST)" --release --locked --offline
	"$(OUTPUT)/bin/hyper-brand-elf" --check \
		"$(TEST_OUTPUT)/rust/aarch64-unknown-none/release/hyper-rust-smoke"
	test -s "$(OUTPUT)/share/hyper/manifest"

test-publication: verify-sources
	sh tests/sysroot-publication.sh "$(ABI_SOURCE)" "$(LIB_SOURCE)" "$(RUST_SOURCE)"

clean:
	rm -rf target
