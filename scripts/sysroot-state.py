#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

"""Content-based validation for an installed SDK; never trust a timestamp stamp."""
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import stat
import subprocess
import sys

EXCLUDED = {".git", "target", "__pycache__"}
ENVIRONMENT = (
    "CLANG", "HOST_CC", "LLVM_AR", "LLVM_RANLIB", "HYPER_LD",
    "HYPER_ARCH", "HYPER_CARGO_DRIVER", "HYPER_SDK_VERSION", "HYPER_SDK_SOURCE_REVISION",
    "CFLAGS", "CPPFLAGS", "ASMFLAGS", "LDFLAGS", "CMAKE_GENERATOR",
    "CMAKE_TOOLCHAIN_FILE", "CMAKE_PREFIX_PATH", "SDKROOT", "MACOSX_DEPLOYMENT_TARGET",
)


def digest(path):
    with Path(path).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def tree(path, exclude=()):
    path = Path(path)
    if path.is_file():
        return {path.name: [digest(path), stat.S_IMODE(path.stat().st_mode)]}
    result = {}
    for directory, directories, files in os.walk(path, followlinks=True):
        resolved = Path(directory).resolve()
        for parent in Path(directory).parents:
            if parent == path.parent:
                break
            if parent.resolve() == resolved:
                raise ValueError(f"cyclic build input symlink: {directory}")
        directories[:] = sorted(name for name in directories if name not in EXCLUDED)
        for name in sorted(files):
            source = Path(directory) / name
            relative = source.relative_to(path).as_posix()
            if relative in exclude:
                continue
            result[relative] = [
                digest(source), stat.S_IMODE(source.stat().st_mode),
                os.readlink(source) if source.is_symlink() else None,
            ]
    return result


def encode(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def tool_identity(name):
    path = shutil.which(name)
    if path is None:
        raise FileNotFoundError(f"required build tool is unavailable: {name}")
    # Resolve symlinks but retain the invocation name (clang/clang++ may share a binary).
    return [path, str(Path(path).resolve()), digest(path)]


def inputs(paths):
    tools = {variable: tool_identity(os.environ.get(variable, name)) for variable, name in (
        ("CLANG", "clang"), ("HOST_CC", "clang"), ("LLVM_AR", "llvm-ar"),
        ("LLVM_RANLIB", "llvm-ranlib"), ("HYPER_LD", "ld.lld"),
        ("HYPER_CARGO_DRIVER", "cargo"), ("CMAKE_COMMAND", "cmake"))}
    # Rustup's proxy binary is not the selected compiler. Include its actual identity.
    tools["rustc"] = subprocess.check_output(["rustc", "-vV"], text=True)
    sources = {str(Path(path).resolve()): tree(path) for path in paths}
    return {"format": 1, "sources": sources, "tools": tools,
            "environment": {key: os.environ.get(key) for key in ENVIRONMENT},
            "platform": platform.platform(), "python": sys.version}


def preserve_times(old, new):
    old, new = Path(old), Path(new)
    for relative, identity in tree(new).items():
        previous = old / relative
        current = new / relative
        if previous.is_file() and not previous.is_symlink():
            info = previous.stat()
            if stat.S_IMODE(info.st_mode) == identity[1] and digest(previous) == identity[0]:
                os.utime(current, ns=(info.st_atime_ns, info.st_mtime_ns))


def main():
    operation, *arguments = sys.argv[1:]
    if operation == "inputs":
        destination, *sources = arguments
        Path(destination).write_bytes(encode(inputs(sources)))
    elif operation == "compare":
        before, after = (json.loads(Path(path).read_bytes()) for path in arguments)
        if before != after:
            for key in before.keys() | after.keys():
                if before.get(key) != after.get(key):
                    if key == "sources":
                        for source in before[key].keys() | after[key].keys():
                            left, right = before[key].get(source, {}), after[key].get(source, {})
                            for name in left.keys() | right.keys():
                                if left.get(name) != right.get(name):
                                    print(f"SDK input changed while building: {source}/{name}", file=sys.stderr)
                    else:
                        print(f"SDK input changed while building: {key}", file=sys.stderr)
            sys.exit(1)
    elif operation == "check":
        output, requested = map(Path, arguments)
        try:
            state = json.loads(Path(str(output) + ".build-state.json").read_bytes())
            valid = state["inputs"] == json.loads(requested.read_bytes()) and state["outputs"] == tree(output)
        except (OSError, ValueError, KeyError):
            valid = False
        sys.exit(0 if valid and output.is_dir() else 1)
    elif operation == "record":
        output, requested, destination = map(Path, arguments)
        Path(destination).write_bytes(encode({"inputs": json.loads(requested.read_bytes()), "outputs": tree(output)}))
    elif operation == "preserve":
        preserve_times(*arguments)
    elif operation == "link-id":
        output, requested = map(Path, arguments)
        state = json.loads(requested.read_bytes())
        # Cargo does not track native archives/linker scripts passed by a custom
        # linker. This explicit identity invalidates consumers when those change.
        identity = {"lib": tree(output / "lib"), "bin": tree(output / "bin"),
                    "tools": state["tools"], "environment": {key: value for key, value in state["environment"].items()
                    if key not in {"HYPER_SDK_VERSION", "HYPER_SDK_SOURCE_REVISION"}}}
        (output / "share/hyper/link-fingerprint").write_text(hashlib.sha256(encode(identity)).hexdigest() + "\n")
        # build-std does not reliably invalidate cached sysroot crates after an
        # installed source override changes. Bind those sources to rustflags.
        std_identity = {"sources": tree(output / "share/hyper/rust-src"),
                        "targets": tree(output / "share/hyper/targets")}
        (output / "share/hyper/std-fingerprint").write_text(
            hashlib.sha256(encode(std_identity)).hexdigest() + "\n")
    else:
        raise SystemExit(f"unknown operation: {operation}")


if __name__ == "__main__":
    main()
