<!--
SPDX-FileCopyrightText: 2026 roolrz
SPDX-License-Identifier: Apache-2.0
-->

# HypeR Toolchain

HypeR Toolchain turns LLVM/Clang and the in-tree Native ABI and runtime sources
into a consumable Native application SDK. It owns
compiler-driver defaults, target profiles, sysroot assembly, and the dynamic
PIE link and startup contract.

The top-level build owns product composition. System applications consume only
the assembled SDK output and do not reach back into these source directories.

## Current scope

- AArch64 freestanding C and `no_std` Rust compilation with dynamic PIE linking
  through `hyper-clang` and `hyper-cargo`, shared-object linking through
  `hyper-clang -shared`, and explicitly selectable static PIE linking;
- assembly from one coherent repository revision;
- assembly of Native headers, Rust OS-binding crates, CRT objects,
  `libhyper.a`, `libhyper.so`, the Native interpreter, and the linker contract
  into a sysroot;
- checked HypeR ELF branding after every application link; and
- compile-time and link-time consumer smoke tests.

Native images are little-endian AArch64 `ET_DYN` files. Dynamic executables use
`/lib/ld-hyper-aarch64.so`, eager binding, and `libhyper.so`; static
images use `libhyper.a` and no interpreter. Both modes reject
writable-executable segments and executable stacks. The driver uses LLD, links
at zero for Kernel-selected placement, and validates the completed image before
applying the HypeR OSABI identity.

## Build a sysroot

From the repository root, run:

```sh
make sdk-check
```

The resulting AArch64 SDK is written to `target/sdk/aarch64`. `CLANG`
selects the target compiler, while `HOST_CC` independently selects the host
compiler used to build tools that run during linking. Override `HYPER_LD`,
`LLVM_AR`, and `LLVM_RANLIB` when the corresponding LLVM tools are not
available on `PATH`.

## Rust allocation support

Native `hyper-cargo` builds rebuild `core` and `alloc` with the application's
PIC and abort settings. The distributed bare-metal rlibs are not sufficient
for the Native dynamic/static PIE relocation contract. The driver scopes
`RUSTC_BOOTSTRAP=1` and `-Z build-std=core,alloc` to these builds, using the
repository-pinned Rust compiler and `rust-src` component.

SDK assembly fetches the dependencies pinned by rust-src's Cargo.lock so
subsequent application builds can remain `--offline`. First-time SDK assembly
therefore needs registry access (or a populated Cargo cache). No HypeR
application dependency or Native binary ABI is added by rebuilding these
compiler libraries.

## Component boundaries

- `sdk/abi/` owns machine-visible syscall values and layouts.
- `sdk/lib/` owns Native C runtime semantics.
- `sdk/loader/` owns userspace ELF dependency and relocation policy.
- `sdk/rust/` owns raw and safe Native Rust bindings plus language entry.
- `sdk/toolchain/` owns compiler and SDK assembly mechanics.
- The repository root owns integration and release composition.
- Linux, FreeBSD, and POSIX compatibility remain separate personalities outside
  the Native SDK contract.

## License

Licensed under the Apache License, Version 2.0. See
[the project license](../../LICENSE).

## Sysroot publication

Each build installs into a fresh sibling directory before replacing the output.
Removed headers and libraries therefore cannot survive from a previous build.
An output-specific publication lock rejects concurrent builders; failures before
publication leave the existing sysroot intact, and handled interruptions during
replacement restore the previous directory. Consumers should wait for the build
to finish: the portable directory replacement uses two renames and is not an
atomic switch for concurrent readers. After an uncatchable termination, inspect
the sibling transaction directory and publication lock before retrying.

`make sdk-check` also verifies stale-file removal, preservation of the previous
sysroot after a compiler failure, and exclusion of a concurrent publisher.
