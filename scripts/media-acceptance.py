#!/usr/bin/env python3
"""Opt-in real APFS/exFAT acceptance; never creates/formats/detaches a disk."""
import argparse
import json
import os
from pathlib import Path
import plistlib
import secrets
import subprocess

MARKER = '.kAccounting-Extra-01-media-acceptance.json'

def checked_root(value):
    root = Path(value)
    if not root.is_absolute() or root != root.resolve() or root == Path('/'):
        raise ValueError('root must be an absolute canonical directory without symlinks')
    if not root.is_dir() or not root.name.startswith('kAccounting-Extra-01-media-test-'):
        raise ValueError('use an existing disposable directory named kAccounting-Extra-01-media-test-*')
    project = Path(__file__).resolve().parent.parent
    if root == project or project in root.parents or root in project.parents:
        raise ValueError('source tree is forbidden')
    return root

def filesystem(root, runner=subprocess.run):
    result = runner(['/usr/sbin/diskutil', 'info', '-plist', str(root)], check=True, capture_output=True)
    info = plistlib.loads(result.stdout)
    fs = str(info.get('FilesystemType', '')).lower()
    if fs not in ('apfs', 'exfat'):
        raise ValueError('diskutil must identify APFS or exFAT')
    mount = Path(info.get('MountPoint', '/'))
    if mount == Path('/') or not mount.is_absolute() or not root.is_relative_to(mount) or not info.get('Mounted'):
        raise ValueError('a mounted non-system test volume is required')
    return fs

def prepare(root, expected_fs):
    root = checked_root(root)
    if list(root.iterdir()):
        raise ValueError('test root must be completely empty; nothing was changed')
    if filesystem(root) != expected_fs:
        raise ValueError('filesystem does not match explicit selection')
    stat = root.stat()
    token = secrets.token_hex(32)
    marker = {'format': 1, 'token': token, 'device': stat.st_dev, 'inode': stat.st_ino, 'filesystem': expected_fs}
    with (root / MARKER).open('x') as file:
        json.dump(marker, file)
    return token

def verify(root, token, expected_fs):
    root = checked_root(root)
    marker = root / MARKER
    if marker.is_symlink() or not marker.is_file():
        raise ValueError('ownership marker missing or unsafe')
    data = json.loads(marker.read_text())
    stat = root.stat()
    if data != {'format': 1, 'token': token, 'device': stat.st_dev, 'inode': stat.st_ino, 'filesystem': expected_fs}:
        raise ValueError('ownership token or root identity mismatch')
    if filesystem(root) != expected_fs:
        raise ValueError('mounted filesystem changed')
    if sorted(p.name for p in root.iterdir()) != [MARKER]:
        raise ValueError('only the ownership marker may exist before acceptance')
    return root

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['prepare', 'run'])
    parser.add_argument('--root', required=True)
    parser.add_argument('--filesystem', required=True, choices=['apfs', 'exfat'])
    parser.add_argument('--token')
    args = parser.parse_args()
    if args.action == 'prepare':
        print(prepare(args.root, args.filesystem))
        return
    if not args.token or len(args.token) != 64:
        parser.error('run requires the token returned by prepare')
    root = verify(args.root, args.token, args.filesystem)
    env = dict(os.environ, KACCOUNTING_MEDIA_TEST_ROOT=str(root), KACCOUNTING_MEDIA_TEST_TOKEN=args.token)
    subprocess.run(['cargo', 'test', '--manifest-path', 'backend/Cargo.toml', '--test', 'media_acceptance', '--', '--ignored', '--nocapture'], cwd=Path(__file__).resolve().parent.parent, env=env, check=True)
    print('PASS: ' + args.filesystem + '; encrypted fixture retained in disposable root; remove it manually after inspection')

if __name__ == '__main__':
    main()
