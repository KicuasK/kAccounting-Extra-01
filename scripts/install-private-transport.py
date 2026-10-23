#!/usr/bin/env python3
"""Publish new, disabled root-owned transport files; never start or log in a service."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import plistlib
import sys
import select
import signal
import time

FORMAT = 'kAccounting-Extra-01.private-transport-install.v1'
BOOTSTRAP_FORMAT = 'kAccounting-Extra-01.private-transport-bootstrap-review.v1'


def check_report_status(report):
    if (not isinstance(report, dict)
            or report.get('format') not in ('kAccounting-Extra-01.private-deployment-review.v1', BOOTSTRAP_FORMAT)
            or report.get('example_only') is not False
            or any(report.get(key) is not False for key in ('installed', 'started', 'qualified', 'funnel_enabled'))
            or (report.get('format') == BOOTSTRAP_FORMAT and any(report.get(key) is not False
                for key in ('deployment_ready', 'signed', 'serve_configured', 'login_performed')))):
        raise ValueError('Requires recognized non-example disabled review')

ROOT = Path('/Library/Application Support/kAccounting-Extra-01/private-transport')
MAX_BINARY = 256 * 1024 * 1024


def load_review(bootstrap=False):
    # Only called without root privileges, including the dropped child.
    if os.geteuid() == 0:
        raise ValueError('Review modules must never load with root privileges')
    import importlib.util
    spec = importlib.util.spec_from_file_location('install_review', Path(__file__).with_name('prepare-private-transport.py' if bootstrap else 'prepare-private-deployment.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def file_identity(value):
    return (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns)


def directory_fd(path, owner):
    """Walk absolute parents through pinned descriptors; refuse links and writable ancestors."""
    path = Path(path)
    if not path.is_absolute() or '..' in path.parts:
        raise ValueError('Absolute canonical installation parent required')
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for component in path.parts[1:]:
            child = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
            metadata = os.fstat(fd)
            if metadata.st_uid != owner or stat.S_IMODE(metadata.st_mode) & 0o022:
                raise ValueError('Installation ancestors must have trusted ownership and modes')
        return fd
    except BaseException:
        os.close(fd)
        raise


def source_bytes(pinned):
    fd = os.open(pinned['path'], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        before = os.fstat(fd)
        if (not stat.S_ISREG(before.st_mode) or before.st_nlink != 1
                or (before.st_dev, before.st_ino, before.st_size) !=
                (pinned['device'], pinned['inode'], pinned['bytes'])
                or oct(stat.S_IMODE(before.st_mode)) != pinned['mode']):
            raise ValueError('Transport input identity changed')
        contents = bytearray()
        while True:
            block = os.read(fd, min(1024 * 1024, MAX_BINARY + 1 - len(contents)))
            if not block:
                break
            contents.extend(block)
            if len(contents) > MAX_BINARY:
                raise ValueError('Transport input exceeds bound')
        after = os.fstat(fd)
        if (file_identity(before) != file_identity(after)
                or hashlib.sha256(contents).hexdigest() != pinned['sha256']):
            raise ValueError('Transport input changed during read')
        return bytes(contents)
    finally:
        os.close(fd)


def emit(fd, name, contents, mode, owner):
    target = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode, dir_fd=fd)
    try:
        os.fchmod(target, mode)
        os.fchown(target, owner, 0 if owner == 0 else os.getgid())
        with os.fdopen(os.dup(target), 'wb') as stream:
            stream.write(contents)
            stream.flush()
            os.fsync(stream.fileno())
        metadata = os.fstat(target)
        if metadata.st_nlink != 1 or metadata.st_uid != owner or metadata.st_gid != (0 if owner == 0 else os.getgid()) or stat.S_IMODE(metadata.st_mode) != mode:
            raise ValueError('Installed file ownership or mode mismatch')
    finally:
        os.close(target)


def publish(report, expected, destination, owner, validate, guard=None):
    """Internal filesystem boundary; destination/owner are fixed by the public CLI."""
    destination = Path(destination)
    binaries = {name: source_bytes(report['transport'][name]) for name in ('tailscaled', 'tailscale')}
    validate()
    try:
        parent = directory_fd(destination.parent, owner)
    except FileNotFoundError:
        # Create only the fixed application's immediate parent; never arbitrary ancestors.
        if destination != ROOT:
            raise
        ancestor = directory_fd(destination.parent.parent, owner)
        try:
            os.mkdir(destination.parent.name, 0o700, dir_fd=ancestor)
            created = os.open(destination.parent.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=ancestor)
            try:
                os.fchown(created, owner, 0 if owner == 0 else os.getgid())
            finally:
                os.close(created)
            os.fsync(ancestor)
        finally:
            os.close(ancestor)
        parent = directory_fd(destination.parent, owner)
    try:
        os.mkdir(destination.name, 0o700, dir_fd=parent)  # Never adopt any existing name.
        root = os.open(destination.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
        try:
            os.fchmod(root, 0o700)
            os.fchown(root, owner, 0 if owner == 0 else os.getgid())
            for name in ('bin', 'state'):
                os.mkdir(name, 0o700, dir_fd=root)
                child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=root)
                try:
                    os.fchown(child, owner, 0 if owner == 0 else os.getgid())
                finally:
                    os.close(child)
            bin_fd = os.open('bin', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=root)
            try:
                for name, contents in binaries.items():
                    emit(bin_fd, name, contents, 0o700, owner)
                if guard is not None:
                    emit(bin_fd, 'private-transport-guard.py', guard, 0o700, owner)
                os.fsync(bin_fd)
            finally:
                os.close(bin_fd)
            validate()
            disabled_plan = {'Label': 'local.kAccounting-Extra-01.private-transport',
                'Disabled': True, 'UserName': 'root', 'RunAtLoad': False, 'KeepAlive': False,
                'Umask': 0o077, 'WorkingDirectory': str(ROOT),
                'ProgramArguments': ['/usr/bin/python3', '-I', '-S', str(ROOT / 'bin/private-transport-guard.py')]}
            emit(root, 'disabled-launchdaemon-review.plist', plistlib.dumps(disabled_plan), 0o600, owner)
            receipt = {'format': FORMAT, 'review_sha256': expected,
                       'destination': str(destination), 'files_published': True,
                       'installed': True, 'started': False, 'qualified': False,
                       'launchdaemon_installed': False, 'control_parent_created': False,
                       'login_performed': False, 'activation_implemented': False,
                       'boot_control_recreation_implemented': guard is not None,
                       'guard_sha256': hashlib.sha256(guard).hexdigest() if guard is not None else None, 'transport_sha256': {
                           name: hashlib.sha256(contents).hexdigest() for name, contents in binaries.items()}}
            emit(root, 'install-receipt.json', (json.dumps(receipt, sort_keys=True) + '\n').encode(), 0o600, owner)
            os.fsync(root)
        finally:
            os.close(root)
        os.fsync(parent)
    finally:
        os.close(parent)
    return receipt


def validated_report(path, expected, drop_identity=None):
    if drop_identity is None:
        # Dispatch only after bounded, digest-pinned read without importing modules.
        fd = os.open(Path(path) / 'review.json', os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            metadata = os.fstat(fd)
            raw = os.read(fd, 1024 * 1024 + 1)
            if (not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1
                    or len(raw) > 1024 * 1024 or hashlib.sha256(raw).hexdigest() != expected
                    or file_identity(metadata) != file_identity(os.fstat(fd))):
                raise ValueError('Unsafe review dispatch')
            document = json.loads(raw)
            check_report_status(document)
        finally:
            os.close(fd)
        review = load_review(document['format'] == BOOTSTRAP_FORMAT)
        result = review.revalidate_inputs(path, expected)
        _, raw = review.identity(Path(path) / 'review.json', 1024 * 1024)
        if hashlib.sha256(raw).hexdigest() != expected:
            raise ValueError('Review changed')
        return result, json.loads(raw, object_pairs_hook=review.bundle.unique)
    # Root never overrides validators or executes a caller-provided subprocess.
    # The fork inherits this already loaded tool, permanently drops privilege,
    # then applies the ordinary review contract to the operator-owned inputs.
    read_fd, write_fd = os.pipe()
    pid = os.fork()
    if pid == 0:
        os.close(read_fd)
        try:
            uid, gid = drop_identity
            os.setgroups([])
            os.setgid(gid)
            os.setuid(uid)
            payload = json.dumps(validated_report(path, expected)).encode()
            with os.fdopen(write_fd, 'wb') as stream:
                stream.write(payload)
            os._exit(0)
        except BaseException:
            os._exit(1)
    os.close(write_fd)
    chunks = bytearray()
    deadline = time.monotonic() + 60
    try:
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([read_fd], [], [], remaining)[0]:
                raise ValueError('Unprivileged validation timed out')
            block = os.read(read_fd, 65536)
            if not block:
                break
            chunks.extend(block)
            if len(chunks) > 4 * 1024 * 1024:
                raise ValueError('Validation response exceeds bound')
    except BaseException:
        os.kill(pid, signal.SIGKILL)
        os.waitpid(pid, 0)
        raise
    finally:
        os.close(read_fd)
    while True:
        finished, status = os.waitpid(pid, os.WNOHANG)
        if finished:
            break
        if time.monotonic() >= deadline:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
            raise ValueError('Unprivileged validation exit timed out')
        time.sleep(0.01)
    if status != 0:
        raise ValueError('Unprivileged input validation failed')
    raw = bytes(chunks)
    return json.loads(raw)


def install(path, expected, apply=False, guard_sha256=None):
    import re
    if not isinstance(expected, str) or not re.fullmatch('[0-9a-f]{64}', expected):
        raise ValueError('Explicit review SHA-256 required')
    guard = None
    if guard_sha256 is not None:
        if not re.fullmatch('[0-9a-f]{64}', guard_sha256):
            raise ValueError('Explicit guard SHA-256 required')
        guard_path = Path(__file__).with_name('private-transport-guard.py')
        fd = os.open(guard_path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            before = os.fstat(fd)
            guard = os.read(fd, 65536 + 1)
            if (not stat.S_ISREG(before.st_mode) or before.st_nlink != 1
                    or not 0 < len(guard) <= 65536 or file_identity(before) != file_identity(os.fstat(fd))
                    or hashlib.sha256(guard).hexdigest() != guard_sha256):
                raise ValueError('Guard identity or digest mismatch')
        finally:
            os.close(fd)
    identity = None
    if apply:
        if guard is None:
            raise ValueError('Administrator apply requires pinned guard digest')
        if sys.platform != 'darwin' or os.getuid() != 0 or os.geteuid() != 0:
            raise ValueError('Apply requires administrator execution on macOS')
        if not sys.flags.isolated:
            raise ValueError('Administrator apply must use Python isolated mode (-I)')
        uid, gid = int(os.environ.get('SUDO_UID', '-1')), int(os.environ.get('SUDO_GID', '-1'))
        if uid <= 0 or gid < 0:
            raise ValueError('Apply requires an identified non-root sudo operator')
        identity = (uid, gid)
    result, report = validated_report(path, expected, identity)
    if apply:
        fd = os.open(Path(path) / 'review.json', os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            metadata = os.fstat(fd)
            if not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1 or metadata.st_uid != identity[0]:
                raise ValueError('Unsafe operator review')
            raw = os.read(fd, 1024 * 1024 + 1)
            if (hashlib.sha256(raw).hexdigest() != expected or len(raw) > 1024 * 1024
                    or file_identity(metadata) != file_identity(os.fstat(fd))):
                raise ValueError('Root review digest mismatch')
            pinned_report = json.loads(raw)
            check_report_status(pinned_report)
            if pinned_report != report:
                raise ValueError('Unprivileged report mismatch')
        finally:
            os.close(fd)
    if not apply:
        return {**result, 'operation': 'plan', 'destination': str(ROOT),
                'requires_administrator_apply': True, 'will_start': False, 'guard_sha256': guard_sha256,
                'launchdaemon_installed': False, 'control_parent_created': False}
    def validate_again():
        current_result, current_report = validated_report(path, expected, identity)
        if current_report != report or current_result != result:
            raise ValueError('Review changed during installation')
    return publish(report, expected, ROOT, 0, validate_again, guard)


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument('--review', required=True)
    parser.add_argument('--expected-review-sha256', required=True)
    group = parser.add_mutually_exclusive_group()
    group.add_argument('--plan', action='store_true', help='Default: read-only source revalidation')
    group.add_argument('--apply', action='store_true', help='Administrator-only new disabled file publication')
    parser.add_argument('--guard-sha256', required=True, help='Independently retained SHA-256 of companion immutable boot guard')
    args = parser.parse_args()
    print(json.dumps(install(args.review, args.expected_review_sha256, args.apply, args.guard_sha256), sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, TypeError, KeyError):
        print('Transport installation refused. No service started. Any incomplete new directory is preserved for operator review; it is never reused or deleted.', file=sys.stderr)
        sys.exit(1)
