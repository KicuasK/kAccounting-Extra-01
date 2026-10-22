#!/usr/bin/env python3
"""Prepare/verify a private DEVELOPMENT runtime bundle; never execute/install it."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import struct
import sys

SOURCE = Path(__file__).resolve().parents[1]
FORMAT = 'kAccounting-Extra-01.development-runtime.v1'
MAX_BINARY = 256 * 1024 * 1024
MAX_HELPER = 32 * 1024 * 1024
MAX_ASSET = 32 * 1024 * 1024
MAX_TOTAL = 512 * 1024 * 1024
MAX_FILES = 130
TARGETS = {'macos-arm64': 0x0100000c, 'macos-x86_64': 0x01000007}


class Refused(ValueError):
    pass


def encoded(value):
    return (json.dumps(value, ensure_ascii=True, sort_keys=True, indent=2) + '\n').encode()


def canonical(path):
    p = Path(path)
    if not p.is_absolute() or '..' in p.parts or p.resolve(strict=True) != p:
        raise Refused('Path must be absolute, canonical and free of symlinks.')
    current = Path(p.anchor)
    for part in p.parts[1:]:
        current /= part
        if stat.S_ISLNK(current.lstat().st_mode):
            raise Refused('Symlink component refused.')
    return p


def regular(path, maximum, executable=False):
    p = canonical(path)
    m = p.lstat()
    if (not stat.S_ISREG(m.st_mode) or m.st_nlink != 1 or m.st_uid != os.getuid()
            or m.st_mode & 0o022 or m.st_size > maximum or m.st_size == 0
            or (executable and not m.st_mode & 0o100)):
        raise Refused('Unsafe, empty, hardlinked or oversized input file.')
    return m


def identity(m):
    return m.st_dev, m.st_ino, m.st_size, m.st_mtime_ns, m.st_mode, m.st_nlink


def read_checked(path, maximum):
    before = regular(path, maximum)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        if identity(os.fstat(fd)) != identity(before):
            raise Refused('Input identity changed before read.')
        chunks, size = [], 0
        while True:
            chunk = os.read(fd, min(65536, maximum + 1 - size))
            if not chunk:
                break
            size += len(chunk)
            if size > maximum:
                raise Refused('Input grew beyond capacity.')
            chunks.append(chunk)
        if identity(os.fstat(fd)) != identity(before) or identity(Path(path).lstat()) != identity(before):
            raise Refused('Input identity changed during read.')
        return b''.join(chunks)
    finally:
        os.close(fd)


def macho(path, maximum, target):
    regular(path, maximum, True)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        header = os.read(fd, 8)
    finally:
        os.close(fd)
    if len(header) != 8 or header[:4] != b'\xcf\xfa\xed\xfe' or struct.unpack('<I', header[4:])[0] != TARGETS[target]:
        raise Refused('Expected a thin 64-bit Mach-O matching the stated target.')


def safe_directory(path, private=False):
    p = canonical(path)
    m = p.lstat()
    if (not stat.S_ISDIR(m.st_mode) or m.st_uid != os.getuid() or m.st_mode & 0o022
            or (private and stat.S_IMODE(m.st_mode) != 0o700)):
        raise Refused('Unsafe directory owner or permissions.')
    return m


def frontend_inputs(root):
    safe_directory(root)
    result = []
    for directory, dirs, files in os.walk(root, followlinks=False):
        base = Path(directory)
        safe_directory(base)
        for name in dirs:
            if (base / name).relative_to(root).as_posix() != 'assets':
                raise Refused('Unknown frontend directory.')
            safe_directory(base / name)
        for name in files:
            path = base / name
            relative = path.relative_to(root).as_posix()
            if relative != 'index.html' and not re.fullmatch(r'assets/[A-Za-z0-9._-]+\.(?:js|css)', relative):
                raise Refused('Unknown frontend file.')
            regular(path, 1024 * 1024 if relative == 'index.html' else MAX_ASSET)
            result.append((path, 'frontend/' + relative, 0o600))
            if len(result) > MAX_FILES - 2:
                raise Refused('Too many frontend files.')
    if 'frontend/index.html' not in [name for _, name, _ in result]:
        raise Refused('Frontend entry point is missing.')
    return result


def create_file(path, data, mode):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    try:
        with os.fdopen(fd, 'wb', closefd=False) as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
    finally:
        os.close(fd)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def build(binary, frontend, output, target, revision, helper=None):
    if target not in TARGETS or not re.fullmatch('[0-9a-f]{40}', revision):
        raise Refused('Target and exact source revision are required.')
    binary, frontend = canonical(binary), canonical(frontend)
    macho(binary, MAX_BINARY, target)
    inputs = [(binary, 'bin/kaccounting-backend', 0o700)] + frontend_inputs(frontend)
    if helper:
        helper = canonical(helper)
        macho(helper, MAX_HELPER, target)
        inputs.append((helper, 'bin/StoragePicker', 0o700))
    output = Path(output)
    if not output.is_absolute() or '..' in output.parts or not output.name:
        raise Refused('New output must be absolute.')
    parent = canonical(output.parent)
    parent_identity = safe_directory(parent, True)
    if SOURCE == output or SOURCE in output.parents or output in SOURCE.parents:
        raise Refused('Bundle output must be outside the source checkout.')
    # Exclusive new directory; failures intentionally preserve partial work.
    output.mkdir(mode=0o700)
    root_identity = output.lstat()
    files, total = {}, 0
    for source, relative, mode in sorted(inputs, key=lambda item: item[1]):
        if identity(parent.lstat())[:2] != identity(parent_identity)[:2] or identity(output.lstat())[:2] != identity(root_identity)[:2]:
            raise Refused('Output directory identity changed.')
        maximum = MAX_BINARY if relative == 'bin/kaccounting-backend' else MAX_HELPER if relative == 'bin/StoragePicker' else MAX_ASSET
        data = read_checked(source, maximum)
        total += len(data)
        if total > MAX_TOTAL:
            raise Refused('Bundle exceeds total capacity.')
        destination = output / relative
        current = output
        for part in Path(relative).parts[:-1]:
            current /= part
            current.mkdir(mode=0o700, exist_ok=True)
            safe_directory(current, True)
        create_file(destination, data, mode)
        files[relative] = {'sha256': digest(data), 'bytes': len(data), 'mode': oct(mode)}
    manifest = {'format': FORMAT, 'project': 'kAccounting-Extra-01', 'kind': 'development-runtime',
                'target': target, 'source_revision': revision, 'source_revision_role': 'declared-base-not-build-attestation', 'signing': 'unverified-development-build',
                'files': files, 'startup': {'command': 'serve-chooser',
                    'arguments': ['<PRIVATE_BOOK_COLLECTION>', '<BUNDLE>/frontend'],
                    'requires_explicit_book_selection': True,
                    'existing_books_require_original_keychain_material': True,
                    'native_helper': 'bin/StoragePicker' if helper else None}}
    manifest_bytes = encoded(manifest)
    create_file(output / 'manifest.json', manifest_bytes, 0o600)
    expected = digest(manifest_bytes)
    # This final marker is the only completion publication. Partial directories
    # are never adopted, overwritten or treated as complete.
    create_file(output / 'complete.json', encoded({'format': FORMAT, 'manifest_sha256': expected}), 0o600)
    for directory, _, _ in os.walk(output, topdown=False):
        fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try: os.fsync(fd)
        finally: os.close(fd)
    verify(output, expected)
    return {'output': str(output), 'manifest_sha256': expected, 'file_count': len(files), 'complete': True}


def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result: raise Refused('Duplicate JSON field.')
        result[key] = value
    return result


def verify(root, expected=None):
    root = canonical(root)
    safe_directory(root, True)
    raw = read_checked(root / 'manifest.json', 1024 * 1024)
    marker = json.loads(read_checked(root / 'complete.json', 4096), object_pairs_hook=unique)
    manifest = json.loads(raw, object_pairs_hook=unique)
    if (not isinstance(marker, dict) or set(marker) != {'format', 'manifest_sha256'}
            or marker['format'] != FORMAT or marker['manifest_sha256'] != digest(raw)
            or (expected is not None and expected != digest(raw))):
        raise Refused('Completion or trusted manifest digest mismatch.')
    required = {'format', 'project', 'kind', 'target', 'source_revision', 'source_revision_role', 'signing', 'files', 'startup'}
    if (not isinstance(manifest, dict) or set(manifest) != required or manifest['format'] != FORMAT
            or manifest['project'] != 'kAccounting-Extra-01' or manifest['kind'] != 'development-runtime'
            or manifest['source_revision_role'] != 'declared-base-not-build-attestation'
            or manifest['target'] not in TARGETS or manifest['signing'] != 'unverified-development-build'
            or not re.fullmatch('[0-9a-f]{40}', manifest['source_revision'])
            or not isinstance(manifest['files'], dict) or not 2 <= len(manifest['files']) <= MAX_FILES):
        raise Refused('Invalid development bundle manifest.')
    files = manifest['files']
    if 'bin/kaccounting-backend' not in files or 'frontend/index.html' not in files:
        raise Refused('Required runtime files absent.')
    allowed = set(files) | {'manifest.json', 'complete.json'}
    actual, total = set(), 0
    for directory, dirs, names in os.walk(root, followlinks=False):
        base = Path(directory)
        safe_directory(base, True)
        for name in dirs:
            rel = (base / name).relative_to(root).as_posix()
            if rel not in {'bin', 'frontend', 'frontend/assets'}: raise Refused('Unknown bundle directory.')
            safe_directory(base / name, True)
        for name in names:
            rel = (base / name).relative_to(root).as_posix()
            if rel not in allowed: raise Refused('Unknown bundle file.')
            actual.add(rel)
            p = base / name
            if rel in {'manifest.json', 'complete.json'}:
                regular(p, 1024 * 1024)
                if stat.S_IMODE(p.lstat().st_mode) != 0o600: raise Refused('Metadata mode changed.')
                continue
            mode = 0o700 if rel in {'bin/kaccounting-backend', 'bin/StoragePicker'} else 0o600
            if mode == 0o600 and rel != 'frontend/index.html' and not re.fullmatch(r'frontend/assets/[A-Za-z0-9._-]+\.(?:js|css)', rel): raise Refused('Unsafe bundle member name.')
            info = files[rel]
            if (not isinstance(info, dict) or set(info) != {'sha256', 'bytes', 'mode'} or info['mode'] != oct(mode) or not isinstance(info['bytes'], int) or isinstance(info['bytes'], bool) or info['bytes'] <= 0 or not isinstance(info['sha256'], str) or not re.fullmatch('[0-9a-f]{64}', info['sha256'])): raise Refused('Invalid member descriptor.')
            maximum = MAX_BINARY if rel == 'bin/kaccounting-backend' else MAX_HELPER if rel == 'bin/StoragePicker' else 1024 * 1024 if rel == 'frontend/index.html' else MAX_ASSET
            regular(p, maximum)
            if stat.S_IMODE(p.lstat().st_mode) != mode: raise Refused('Member permissions changed.')
            data = read_checked(p, maximum)
            total += len(data)
            if len(data) != info['bytes'] or digest(data) != info['sha256'] or total > MAX_TOTAL: raise Refused('Member corruption or capacity exceeded.')
            if mode == 0o700: macho(p, maximum, manifest['target'])
    if actual != allowed: raise Refused('Missing bundle files.')
    if manifest['startup'] != {'command': 'serve-chooser', 'arguments': ['<PRIVATE_BOOK_COLLECTION>', '<BUNDLE>/frontend'], 'requires_explicit_book_selection': True, 'existing_books_require_original_keychain_material': True, 'native_helper': 'bin/StoragePicker' if 'bin/StoragePicker' in files else None}: raise Refused('Unexpected startup metadata.')
    return {'complete': True, 'manifest_sha256': digest(raw), 'file_count': len(files), 'target': manifest['target']}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--verify')
    p.add_argument('--expected-manifest-sha256')
    p.add_argument('--binary'); p.add_argument('--frontend'); p.add_argument('--output'); p.add_argument('--native-helper')
    p.add_argument('--target', choices=sorted(TARGETS), default='macos-arm64'); p.add_argument('--revision')
    args = p.parse_args()
    if args.verify:
        if any([args.binary, args.frontend, args.output, args.native_helper, args.revision]): p.error('Verify cannot include build inputs.')
        result = verify(args.verify, args.expected_manifest_sha256)
    else:
        if not all([args.binary, args.frontend, args.output, args.revision]) or args.expected_manifest_sha256: p.error('Build needs binary, frontend, NEW output and exact revision.')
        result = build(args.binary, args.frontend, args.output, args.target, args.revision, args.native_helper)
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    try: main()
    except (OSError, ValueError, TypeError, KeyError):
        print('Development bundle refused: invalid input, integrity, permissions or capacity; any partial output was preserved.', file=sys.stderr)
        sys.exit(1)
