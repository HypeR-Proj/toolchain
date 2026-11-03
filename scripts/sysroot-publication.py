#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

"""Inspectable SDK publication ownership on local POSIX filesystems."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import secrets
import socket
import stat
import sys
import time


class PublicationError(Exception):
    pass


def paths(output):
    output = Path(output).absolute()
    if output.name in ('', '.', '..') or output == Path('/'):
        raise PublicationError('unsafe SDK output path')
    output.parent.mkdir(parents=True, exist_ok=True)
    output = output.parent.resolve() / output.name
    return output, Path(str(output) + '.publish-lock'), Path(str(output) + '.publish-guard')


def guard(path):
    fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    if not stat.S_ISREG(os.fstat(fd).st_mode):
        os.close(fd)
        raise PublicationError('publication guard is not a regular file')
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        os.close(fd)
        return None
    return fd


def read_owner(output, lock):
    if lock.is_symlink() or not lock.is_dir():
        raise PublicationError('unknown publication owner: not a regular lock directory')
    try:
        if sorted(p.name for p in lock.iterdir()) != ['owner.json']:
            raise ValueError('unexpected lock contents')
        owner_file = lock / 'owner.json'
        if owner_file.is_symlink():
            raise ValueError('owner record is a symbolic link')
        owner = json.loads(owner_file.read_text())
        if (owner['version'] != 1 or owner['output'] != str(output)
                or not isinstance(owner['pid'], int) or owner['pid'] <= 0
                or not isinstance(owner['token'], str) or len(owner['token']) != 32
                or not isinstance(owner['host'], str)
                or not isinstance(owner['started_at_ns'], int)
                or (owner.get('transaction') is not None
                    and not isinstance(owner['transaction'], str))):
            raise ValueError('invalid owner identity')
    except (OSError, ValueError, KeyError, TypeError) as error:
        raise PublicationError(f'unknown publication owner: {error}') from error
    return owner


def write_owner(lock, owner):
    temporary = lock / 'owner.next'
    with temporary.open('x') as stream:
        json.dump(owner, stream, indent=2)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(lock / 'owner.json')


def owned(output, lock, guard_path):
    owner = read_owner(output, lock)
    try:
        fd = int(os.environ['HYPER_PUBLICATION_FD'])
        held = os.fstat(fd)
        expected = guard_path.stat()
        valid = (held.st_dev, held.st_ino) == (expected.st_dev, expected.st_ino)
        valid = valid and owner['pid'] == os.getppid()
        valid = valid and owner['token'] == os.environ['HYPER_PUBLICATION_TOKEN']
    except (OSError, ValueError, KeyError):
        valid = False
    if not valid:
        raise PublicationError('publication operation does not own this SDK lock')
    return owner


def transaction_path(output, value, allow_removed=False):
    transaction = Path(value)
    if (transaction.parent != output.parent or not transaction.name.startswith('.hyper-sysroot.')
            or transaction.is_symlink()
            or (not transaction.is_dir() and not (allow_removed and not transaction.exists()))):
        raise PublicationError('transaction is not a regular SDK staging sibling')
    return transaction


def release(lock):
    (lock / 'owner.json').unlink()
    lock.rmdir()


def execute(output, lock, guard_path, command):
    fd = guard(guard_path)
    if fd is None:
        raise PublicationError('SDK publisher is active; inspect its owner before retrying')
    try:
        if lock.exists() or lock.is_symlink():
            raise PublicationError('publication record remains; run inspect and recover before retrying')
        lock.mkdir()
        owner = dict(version=1, pid=os.getpid(), host=socket.gethostname(),
                     started_at_ns=time.time_ns(), token=secrets.token_hex(16),
                     output=str(output), command=command, transaction=None)
        write_owner(lock, owner)
        os.set_inheritable(fd, True)
        environment = dict(os.environ, HYPER_PUBLICATION_FD=str(fd),
                           HYPER_PUBLICATION_TOKEN=owner['token'])
        # The publisher itself owns the kernel lock across exec. SIGKILL cannot
        # leave an apparently live lock; descendants inheriting it conservatively
        # prevent recovery until they have also exited.
        try:
            os.execvpe(command[0], command, environment)
        except OSError:
            release(lock)
            raise
    finally:
        os.close(fd)


def inspect_or_recover(action, output, lock, guard_path):
    fd = guard(guard_path)
    if fd is None:
        if action == 'recover':
            raise PublicationError('refusing recovery: SDK publisher is active')
        try:
            owner = read_owner(output, lock)
        except PublicationError:
            owner = None
        print(json.dumps(dict(state='active', owner=owner), indent=2))
        return
    try:
        if not lock.exists() and not lock.is_symlink():
            print(json.dumps(dict(state='idle', output=str(output))))
            return
        owner = read_owner(output, lock)
        if owner['host'] != socket.gethostname():
            raise PublicationError('unknown owner host; automatic recovery requires the original host')
        if action == 'inspect':
            print(json.dumps(dict(state='stale', owner=owner), indent=2))
            return
        transaction = None
        if owner.get('transaction') is not None:
            # Cleanup can be killed after deleting staging but before removing
            # its owner record. A completed output makes that case recoverable.
            transaction = transaction_path(output, owner['transaction'],
                                           allow_removed=output.exists() or output.is_symlink())
            previous = transaction / 'previous'
            if (previous.exists() or previous.is_symlink()) and not (output.exists() or output.is_symlink()):
                previous.rename(output)
        # The guard serializes publishers and all recoverers. Never unlink it:
        # replacing its inode would permit two independent lock owners.
        release(lock)
        print(json.dumps(dict(state='recovered', output=str(output),
                              preserved_transaction=str(transaction) if transaction else None)))
    finally:
        os.close(fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('run', 'verify', 'record', 'release', 'inspect', 'recover'))
    parser.add_argument('output')
    parser.add_argument('arguments', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    output, lock, guard_path = paths(args.output)
    if args.action == 'run':
        if not args.arguments:
            raise PublicationError('run requires a publisher command')
        execute(output, lock, guard_path, args.arguments)
    elif args.action in ('inspect', 'recover'):
        inspect_or_recover(args.action, output, lock, guard_path)
    else:
        owner = owned(output, lock, guard_path)
        if args.action == 'record':
            if len(args.arguments) != 1:
                raise PublicationError('record requires one staging directory')
            owner['transaction'] = str(transaction_path(output, args.arguments[0]))
            write_owner(lock, owner)
        elif args.action == 'release':
            release(lock)


if __name__ == '__main__':
    try:
        main()
    except (PublicationError, OSError) as error:
        print(f'sysroot publication: {error}', file=sys.stderr)
        sys.exit(2)
