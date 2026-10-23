#!/usr/bin/env python3
"""Fixed-path root transport bootstrap; no login, route configuration or cleanup."""
import hashlib
import json
import os
from pathlib import Path
import stat
import sys

ROOT = Path('/Library/Application Support/kAccounting-Extra-01/private-transport')
CONTROL = Path('/private/var/run/kaccounting-private-transport')
MAX_BINARY = 256 * 1024 * 1024
UPGRADE_PROTOCOL = 'kAccounting-Extra-01.transport-version-guard.v1'


def identity(value):
    return (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns,
            value.st_ctime_ns, value.st_mode, value.st_uid, value.st_gid, value.st_nlink)


def directory(path, owner, group):
    path = Path(path)
    if not path.is_absolute() or '..' in path.parts:
        raise ValueError('Canonical absolute directory required')
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        current = Path('/')
        for name in path.parts[1:]:
            current /= name
            child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
            value = os.fstat(fd)
            # macOS owns /private/var/run as root:daemon 0775. Only this
            # exact canonical system parent is an allowed daemon-group boundary.
            allowed = (current == Path('/private/var/run') and owner == 0
                       and value.st_uid == 0 and value.st_gid == 1
                       and stat.S_IMODE(value.st_mode) == 0o775)
            if value.st_uid != owner or (value.st_mode & 0o022 and not allowed):
                raise ValueError('Untrusted transport ancestor')
        return fd
    except BaseException:
        os.close(fd)
        raise


def private_directory(fd, owner, group):
    value = os.fstat(fd)
    if value.st_uid != owner or value.st_gid != group or stat.S_IMODE(value.st_mode) != 0o700:
        raise ValueError('Transport directory must be private and root owned')


def read(fd, name, mode, owner, group, maximum):
    child = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
    try:
        before = os.fstat(child)
        if (not stat.S_ISREG(before.st_mode) or before.st_uid != owner or before.st_gid != group
                or before.st_nlink != 1 or stat.S_IMODE(before.st_mode) != mode
                or not 0 < before.st_size <= maximum):
            raise ValueError('Unsafe installed transport file')
        raw = bytearray()
        while len(raw) <= maximum:
            part = os.read(child, min(65536, maximum + 1 - len(raw)))
            if not part:
                break
            raw.extend(part)
        if (len(raw) > maximum or identity(before) != identity(os.fstat(child))
                or identity(before) != identity(os.stat(name, dir_fd=fd, follow_symlinks=False))):
            raise ValueError('Installed transport file changed')
        return bytes(raw)
    finally:
        os.close(child)


def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('Duplicate receipt key')
        result[key] = value
    return result


def refuse_pending(root_fd):
    try:
        os.stat('version-pending.json', dir_fd=root_fd, follow_symlinks=False)
    except FileNotFoundError:
        return
    raise ValueError('Transport version publication requires reconciliation')


def record_use(root, owner, group):
    """Conservatively mark execution requested before exec; never erase a marker."""
    fd = directory(root, owner, group)
    try:
        private_directory(fd, owner, group)
        refuse_pending(fd)
        raw = read(fd, 'install-receipt.json', 0o600, owner, group, 65536)
        digest = hashlib.sha256(raw).hexdigest()
        name = 'version-use-' + digest + '.json'
        payload = (json.dumps({'format': 'kAccounting-Extra-01.transport-version-use.v1',
                               'receipt_sha256': digest}, sort_keys=True) + '\n').encode()
        try:
            child = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=fd)
        except FileExistsError:
            if read(fd, name, 0o600, owner, group, 4096) != payload:
                raise ValueError('Execution marker changed; preserve for inspection')
        else:
            try:
                os.fchmod(child, 0o600)
                os.fchown(child, owner, group)
                with os.fdopen(os.dup(child), 'wb') as stream:
                    stream.write(payload)
                    stream.flush()
                    os.fsync(stream.fileno())
                os.fsync(fd)
            finally:
                os.close(child)
        refuse_pending(fd)
        if read(fd, 'install-receipt.json', 0o600, owner, group, 65536) != raw:
            raise ValueError('Receipt changed before transport execution')
        return digest
    finally:
        os.close(fd)


def prepare(root, control, owner, group):
    """Internal filesystem check; production entrypoint supplies only fixed constants."""
    root = Path(root)
    control = Path(control)
    root_fd = directory(root, owner, group)
    try:
        private_directory(root_fd, owner, group)
        refuse_pending(root_fd)
        receipt = json.loads(read(root_fd, 'install-receipt.json', 0o600, owner, group, 65536), object_pairs_hook=unique)
        if (receipt.get('format') != 'kAccounting-Extra-01.private-transport-install.v1'
                or receipt.get('destination') != str(root) or receipt.get('installed') is not True):
            raise ValueError('Installed receipt mismatch')
        bins = os.open('bin', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=root_fd)
        state = os.open('state', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=root_fd)
        try:
            private_directory(bins, owner, group)
            private_directory(state, owner, group)
            for name in ('tailscaled', 'tailscale'):
                contents = read(bins, name, 0o700, owner, group, MAX_BINARY)
                if hashlib.sha256(contents).hexdigest() != receipt['transport_sha256'][name]:
                    raise ValueError('Installed transport digest mismatch')
            guard = read(bins, 'private-transport-guard.py', 0o700, owner, group, 65536)
            if hashlib.sha256(guard).hexdigest() != receipt['guard_sha256']:
                raise ValueError('Installed guard digest mismatch')
        finally:
            os.close(bins)
            os.close(state)
    finally:
        os.close(root_fd)
    parent = directory(control.parent, owner, group)
    try:
        created = False
        try:
            os.mkdir(control.name, 0o700, dir_fd=parent)
            created = True
        except FileExistsError:
            pass
        child = os.open(control.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
        try:
            if created:
                metadata = os.fstat(child)
                if metadata.st_uid != owner or stat.S_IMODE(metadata.st_mode) != 0o700 or os.listdir(child):
                    raise ValueError('New control parent changed before normalization')
                # BSD mkdir may inherit daemon gid from /private/var/run.
                # Normalize only the directory created by this invocation.
                os.fchown(child, owner, group)
                os.fchmod(child, 0o700)
            private_directory(child, owner, group)
            # Preserve every unexpected name, including a stale control socket.
            if os.listdir(child):
                raise ValueError('Control parent is not empty; operator inspection required')
            if identity(os.fstat(child)) != identity(os.stat(control.name, dir_fd=parent, follow_symlinks=False)):
                raise ValueError('Control parent changed')
            os.fsync(child)
            os.fsync(parent)
        finally:
            os.close(child)
    finally:
        os.close(parent)
    return [str(root / 'bin/tailscaled'), '--tun=userspace-networking',
            '--statedir=' + str(root / 'state'), '--state=' + str(root / 'state/tailscaled.state'),
            '--socket=' + str(control / 'tailscaled.sock'), '--port=0']


def main():
    if len(sys.argv) != 1 or sys.platform != 'darwin' or os.getuid() != 0 or os.geteuid() != 0:
        raise ValueError('Fixed root macOS bootstrap only')
    os.umask(0o077)
    arguments = prepare(ROOT, CONTROL, 0, 0)
    record_use(ROOT, 0, 0)
    os.chdir(ROOT)
    os.execve(arguments[0], arguments, {'PATH': '/usr/bin:/bin:/usr/sbin:/sbin', 'HOME': '/var/root', 'TMPDIR': '/private/tmp'})


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, KeyError, TypeError):
        print('Private transport bootstrap refused; state and control files preserved.', file=sys.stderr)
        sys.exit(1)
