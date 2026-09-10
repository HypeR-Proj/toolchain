#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

"""Check the machine ABI of actual installed Native binaries."""
import struct
import sys
from pathlib import Path

architecture, *names = sys.argv[1:]
machine = {"aarch64": 183, "riscv64": 243}[architecture]
for name in names:
    data = Path(name).read_bytes()
    assert data[:6] == b"\x7fELF\x02\x01", name
    assert struct.unpack_from("<H", data, 18)[0] == machine, name
    flags = struct.unpack_from("<I", data, 48)[0]
    assert flags == 0 if architecture == "aarch64" else flags & ~1 == 4, name
    offset = struct.unpack_from("<Q", data, 32)[0]
    size, count = struct.unpack_from("<HH", data, 54)
    for index in range(count):
        kind, _, source, _, _, length, memory = struct.unpack_from(
            "<IIQQQQQ", data, offset + index * size
        )
        if kind == 3:
            assert data[source:source + length] == (
                f"/lib/ld-hyper-{architecture}.so\0".encode()
            ), name
        if kind == 7:
            assert memory == 0, name
print(f"verified {architecture} machine, float ABI, interpreter and TLS contracts")
