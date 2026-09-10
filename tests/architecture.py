#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

"""Reject SDK architecture mismatches and incompatible executable float ABIs."""
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile

sdk, image = map(Path, sys.argv[1:])
manifest = dict(line.split("=", 1) for line in (sdk / "share/hyper/manifest").read_text().splitlines())
architecture = manifest["architecture"]
wrong = "riscv64" if architecture == "aarch64" else "aarch64"
for driver, arguments in (("hyper-clang", ["-E", "-x", "c", "-"]),
                          ("hyper-cargo", ["check"])):
    result = subprocess.run([str(sdk / "bin" / driver), *arguments],
                            env=os.environ | {"HYPER_ARCH": wrong},
                            input=b"", capture_output=True)
    assert result.returncode != 0 and b"conflicts" in result.stderr, result.stderr

original = image.read_bytes()
with tempfile.TemporaryDirectory(prefix="hyper-machine-") as temporary:
    path = Path(temporary) / "image"
    cases = [(243, 0), (243, 2), (243, 6), (243, 12), (243, 20), (183, 4), (62, 0)]
    for machine, flags in cases:
        data = bytearray(original)
        struct.pack_into("<H", data, 18, machine)
        struct.pack_into("<I", data, 48, flags)
        path.write_bytes(data)
        result = subprocess.run([str(sdk / "bin/hyper-brand-elf"), "--check", str(path)],
                                capture_output=True)
        assert result.returncode != 0, (machine, flags)
print("verified installed target mismatch and malformed machine/float ABI rejection")
