#!/usr/bin/python3
"""Compile only into a private canonical parent; publish with exclusive link."""
import os
import ctypes
import plistlib
import re
from pathlib import Path
import stat
import subprocess
import sys
import tempfile


def exclusive_rename_at(source_fd, source_name, destination_fd, destination_name):
    # macOS SDK sys/stdio.h: renameatx_np(..., RENAME_EXCL = 0x00000004).
    rename = ctypes.CDLL(None, use_errno=True).renameatx_np
    rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    rename.restype = ctypes.c_int
    if rename(source_fd, os.fsencode(source_name), destination_fd, os.fsencode(destination_name), 0x00000004):
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), destination_name)


def build(output_text, bundle_id='local.kAccounting-Extra-01.StoragePicker'):
    if not re.fullmatch(r'[A-Za-z][A-Za-z0-9-]*(?:\.[A-Za-z][A-Za-z0-9-]*)+', bundle_id):
        raise ValueError('Invalid native helper bundle identifier.')
    output = Path(output_text)
    if not output.is_absolute() or str(output) != output_text or '..' in output.parts:
        raise ValueError('Output must be an absolute normalized file path.')
    parent = output.parent
    if parent.resolve(strict=True) != parent:
        raise ValueError('Output parent must be canonical without symlinks.')
    source = Path(__file__).resolve().parent.parent
    if output == source or source in output.parents:
        raise ValueError('Native helper output must remain outside the source checkout.')
    metadata = parent.lstat()
    if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.getuid() or metadata.st_mode & 0o077:
        raise ValueError('Output parent must be owned by this user and private (mode 0700).')
    if os.path.lexists(output):
        raise ValueError('Refusing to overwrite an existing output.')
    parent_fd = os.open(parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        if (os.fstat(parent_fd).st_dev, os.fstat(parent_fd).st_ino) != (metadata.st_dev, metadata.st_ino):
            raise ValueError('Output parent changed.')
        # Retain owned compile intermediates for review. No recursive deletion.
        stage = Path(tempfile.mkdtemp(prefix='.native-picker-build-', dir=parent))
        binary = stage / 'StoragePicker'
        subprocess.run(['/usr/bin/swiftc', '-O', '-module-cache-path', str(stage / 'module-cache'),
                        '-framework', 'AppKit', str(source / 'native/StoragePicker.swift'),
                        '-o', str(binary)], check=True)
        binary_metadata = binary.lstat()
        if (not stat.S_ISREG(binary_metadata.st_mode) or binary_metadata.st_uid != os.getuid()
                or binary_metadata.st_nlink != 1):
            raise ValueError('Compiler did not produce an owned single-link regular executable.')
        binary_fd = os.open(binary, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            opened = os.fstat(binary_fd)
            if ((opened.st_dev, opened.st_ino) != (binary_metadata.st_dev, binary_metadata.st_ino)
                    or not stat.S_ISREG(opened.st_mode) or opened.st_nlink != 1
                    or opened.st_uid != os.getuid()):
                raise ValueError('Compiler output changed before private permissions were applied.')
            os.fchmod(binary_fd, 0o700)
            os.fsync(binary_fd)
        finally:
            os.close(binary_fd)
        bundle = None
        if output.suffix == '.app':
            bundle = stage / 'StoragePicker.app'
            executable_parent = bundle / 'Contents' / 'MacOS'
            executable_parent.mkdir(parents=True, mode=0o700)
            for directory in [bundle, bundle / 'Contents', executable_parent]:
                directory.chmod(0o700)
            os.link(binary, executable_parent / 'StoragePicker', follow_symlinks=False)
            info = {
                'CFBundleIdentifier': bundle_id, 'CFBundleExecutable': 'StoragePicker',
                'CFBundleName': output.stem, 'CFBundleDisplayName': output.stem,
                'CFBundlePackageType': 'APPL', 'CFBundleVersion': '1',
                'CFBundleShortVersionString': '1.0', 'LSUIElement': False,
                'NSHighResolutionCapable': True, 'LSMinimumSystemVersion': '10.15',
                'NSPrincipalClass': 'NSApplication',
            }
            info_path = bundle / 'Contents' / 'Info.plist'
            with info_path.open('xb') as file:
                plistlib.dump(info, file)
            info_path.chmod(0o600)
        current = parent.lstat()
        opened_parent = os.fstat(parent_fd)
        if (parent.resolve(strict=True) != parent
                or (current.st_dev, current.st_ino) != (metadata.st_dev, metadata.st_ino)
                or (opened_parent.st_dev, opened_parent.st_ino) != (metadata.st_dev, metadata.st_ino)
                or current.st_uid != metadata.st_uid or current.st_mode != metadata.st_mode
                or opened_parent.st_uid != metadata.st_uid or opened_parent.st_mode != metadata.st_mode):
            raise ValueError('Output parent identity or permissions changed during compilation; private stage retained.')
        final_binary = binary.lstat()
        if ((final_binary.st_dev, final_binary.st_ino) != (binary_metadata.st_dev, binary_metadata.st_ino)
                or not stat.S_ISREG(final_binary.st_mode) or final_binary.st_uid != os.getuid()
                or stat.S_IMODE(final_binary.st_mode) != 0o700
                or final_binary.st_nlink != (2 if bundle is not None else 1)):
            raise ValueError('Compiler output changed before publication; private stage retained.')
        # link() creates the final name exclusively: no check-then-overwrite race.
        if bundle is None:
            os.link(binary, output.name, dst_dir_fd=parent_fd, follow_symlinks=False)
        else:
            stage_fd = os.open(stage, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            try:
                exclusive_rename_at(stage_fd, bundle.name, parent_fd, output.name)
            finally:
                os.close(stage_fd)
        os.fsync(parent_fd)
        print('Compiled native helper; no panel opened: ' + str(output))
        if bundle is not None:
            print('Native executable: ' + str(output / 'Contents' / 'MacOS' / 'StoragePicker'))
        print('Private compile intermediates retained: ' + str(stage))
    finally:
        os.close(parent_fd)


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description='Compile only; .app output creates a complete private native application.')
    parser.add_argument('output', help='Canonical private output parent; prefer a new StoragePicker.app path')
    parser.add_argument('--bundle-id', default='local.kAccounting-Extra-01.StoragePicker')
    arguments = parser.parse_args()
    try:
        build(arguments.output, arguments.bundle_id)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
