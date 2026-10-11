#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

"""Exercise bootstrap link rejection through the installed C and Cargo drivers."""

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


sdk, smoke = (Path(value).resolve() for value in sys.argv[1:])
env = dict(os.environ)
env.pop('HYPER_LINK_MODE', None)
env.update(HYPER_SYSROOT=str(sdk), HYPER_RUST_STD='0')


def run(arguments, environment, rejection=None):
    result = subprocess.run(arguments, env=environment, capture_output=True, text=True)
    if rejection is None:
        assert result.returncode == 0, result.stdout + result.stderr
    else:
        assert result.returncode != 0 and rejection in result.stderr, result.stderr


with tempfile.TemporaryDirectory(prefix='hyper-bootstrap-link-') as temporary:
    root = Path(temporary)
    crate = root / 'bootstrap/stage1/probe'
    shutil.copytree(Path(__file__).parent / 'rust-smoke', crate)
    stage_env = env | {'CARGO_MANIFEST_DIR': str(crate)}
    clang = [str(sdk / 'bin/hyper-clang'), str(smoke), '-o', str(root / 'smoke')]
    run(clang, stage_env, 'bootstrap/stage1 only permits static executables')
    run(clang + ['-shared'], stage_env | {'HYPER_LINK_MODE': 'static'},
        'bootstrap/stage1 only permits static executables')
    run(clang + ['-c'], stage_env)
    run(clang + ['-static'], stage_env)
    run([str(sdk / 'bin/hyper-brand-elf'), '--check-static', str(root / 'smoke')], env)

    # The dependency graph and target directory are reused across link modes.
    # Cargo must propagate source ownership to the linker even when invoked
    # from outside the crate, and must not reuse the static result as dynamic.
    cargo_env = env | {'CARGO_TARGET_DIR': str(root / 'cargo')}
    cargo = [str(sdk / 'bin/hyper-cargo'), 'build', '--manifest-path',
             str(crate / 'Cargo.toml'), '--release', '--locked', '--offline']
    run(cargo, cargo_env, 'bootstrap/stage1 only permits static executables')
    run(cargo, cargo_env | {'HYPER_LINK_MODE': 'static'})
    target = {'aarch64': 'aarch64-unknown-none',
              'riscv64': 'riscv64gc-unknown-none-elf'}
    manifest = dict(line.split('=', 1) for line in
                    (sdk / 'share/hyper/manifest').read_text().splitlines())
    image = root / 'cargo' / target[manifest['architecture']] / 'release/hyper-rust-smoke'
    run([str(sdk / 'bin/hyper-brand-elf'), '--check-static', str(image)], env)
    run(cargo, cargo_env, 'bootstrap/stage1 only permits static executables')

print('verified bootstrap static-only C/Cargo linking and link-mode cache separation')
