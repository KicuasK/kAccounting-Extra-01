#!/usr/bin/env python3
"""Prepare a key-free discovery or explicit-selection launcher; never install or start it.

Each application start requires explicit book selection. With built frontend
assets, start the loopback session without opening a book. Otherwise list
candidates only. No remembered or preselected book is accepted.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import stat

SOURCE_ROOT = Path(__file__).resolve().parent.parent
LABEL = "local.kAccounting-Extra-01"


def outside_source(path: Path) -> None:
    if path == SOURCE_ROOT or SOURCE_ROOT in path.parents or path in SOURCE_ROOT.parents:
        raise ValueError("Runtime books and prepared output must remain outside the source tree")


def directory_fd(path: Path) -> int:
    """Walk by directory descriptors so symlink replacement cannot redirect writes."""
    if not path.is_absolute() or ".." in path.parts:
        raise ValueError("Use an absolute path without parent traversal")
    descriptor = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in path.parts[1:]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor)
            os.close(descriptor)
            descriptor = child
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def private_directory(path: Path) -> Path:
    descriptor = directory_fd(path)
    try:
        metadata = os.fstat(descriptor)
        if metadata.st_uid != os.getuid() or stat.S_IMODE(metadata.st_mode) != 0o700:
            raise ValueError("Private directory must be owned by this user with mode 700")
    finally:
        os.close(descriptor)
    actual = path.resolve(strict=True)
    outside_source(actual)
    return actual


def validate_log(path: Path) -> None:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return
    if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid()
            or stat.S_IMODE(metadata.st_mode) & 0o077 or metadata.st_nlink != 1):
        raise ValueError("Existing launcher logs must be private regular single-link files")


def output_parent(output: Path) -> tuple[Path, int]:
    if not output.is_absolute() or output.suffix != ".plist" or ".." in output.parts:
        raise ValueError("--output must be an absolute new .plist file without traversal")
    if any(part.casefold() in {"launchagents", "launchdaemons"} for part in output.parts):
        raise ValueError("Prepare outside LaunchAgents/LaunchDaemons; installation is a separate action")
    parent = private_directory(output.parent)
    descriptor = directory_fd(parent)
    metadata = os.fstat(descriptor)
    if metadata.st_uid != os.getuid() or stat.S_IMODE(metadata.st_mode) != 0o700:
        os.close(descriptor)
        raise ValueError("Output parent changed or is not private")
    try:
        os.stat(output.name, dir_fd=descriptor, follow_symlinks=False)
    except FileNotFoundError:
        return parent / output.name, descriptor
    except BaseException:
        os.close(descriptor)
        raise
    else:
        os.close(descriptor)
        raise ValueError("Existing output is preserved; choose a new .plist file")


def executable_identity(path: Path, maximum_size: int | None = None) -> dict:
    actual = regular(path, executable=True)
    parent = directory_fd(actual.parent)
    try:
        descriptor = os.open(actual.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
    finally:
        os.close(parent)
    try:
        before = os.fstat(descriptor)
        if (not stat.S_ISREG(before.st_mode) or before.st_uid != os.getuid()
                or before.st_mode & (0o022 | stat.S_ISUID | stat.S_ISGID)
                or not before.st_mode & stat.S_IXUSR):
            raise ValueError("Executable must be owned by this user and not writable by others")
        if maximum_size is not None and before.st_size > maximum_size:
            raise ValueError("Executable exceeds its bounded preflight size")
        digest = hashlib.sha256()
        total = 0
        while True:
            amount = 1024 * 1024 if maximum_size is None else min(1024 * 1024, maximum_size + 1 - total)
            chunk = os.read(descriptor, amount)
            if not chunk:
                break
            total += len(chunk)
            if maximum_size is not None and total > maximum_size:
                raise ValueError("Executable grew beyond its bounded preflight size")
            digest.update(chunk)
        after = os.fstat(descriptor)
        named = actual.stat()
        identity = lambda item: (item.st_dev, item.st_ino, item.st_size, item.st_mtime_ns, item.st_ctime_ns)
        if (identity(before) != identity(after) or identity(after) != identity(named)
                or (maximum_size is not None and (after.st_size > maximum_size or total > maximum_size))):
            raise ValueError("Executable changed during preflight")
        return {"sha256": digest.hexdigest(), "size": after.st_size, "device": after.st_dev, "inode": after.st_ino}
    finally:
        os.close(descriptor)


def regular(path: Path, executable: bool = False) -> Path:
    if not path.is_absolute() or any(part.is_symlink() for part in [path, *path.parents]):
        raise ValueError("Use an absolute regular-file path without a symlink")
    actual = path.resolve(strict=True)
    if not actual.is_file() or (executable and not os.access(actual, os.X_OK)):
        raise ValueError("The selected binary or file is unavailable")
    return actual


def unique_fields(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("Duplicate private configuration field")
        value[key] = item
    return value


def private_configuration_identity(path: Path) -> dict:
    if (not path.is_absolute() or ".." in path.parts or path.resolve(strict=True) != path):
        raise ValueError("Private configuration must be canonical without symlinks or traversal")
    outside_source(path)
    private_directory(path.parent)
    parent = directory_fd(path.parent)
    try:
        directory = os.fstat(parent)
        if directory.st_uid != os.getuid() or stat.S_IMODE(directory.st_mode) != 0o700:
            raise ValueError("Private configuration parent changed or is not private")
        descriptor = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
        try:
            before = os.fstat(descriptor)
            if (not stat.S_ISREG(before.st_mode) or before.st_uid != os.getuid()
                    or stat.S_IMODE(before.st_mode) != 0o600 or before.st_nlink != 1
                    or before.st_size > 16384):
                raise ValueError("Private configuration must be an owned single-link mode-600 file of at most 16 KiB")
            contents = bytearray()
            while len(contents) <= 16384:
                chunk = os.read(descriptor, min(65536, 16385 - len(contents)))
                if not chunk:
                    break
                contents.extend(chunk)
            after = os.fstat(descriptor)
            named = os.stat(path.name, dir_fd=parent, follow_symlinks=False)
            identity = lambda item: (item.st_dev, item.st_ino, item.st_size, item.st_mtime_ns, item.st_ctime_ns, item.st_uid, item.st_mode, item.st_nlink)
            if len(contents) > 16384 or identity(before) != identity(after) or identity(after) != identity(named):
                raise ValueError("Private configuration changed during preflight")
            current_parent = path.parent.lstat()
            if (current_parent.st_dev, current_parent.st_ino, current_parent.st_uid, current_parent.st_mode) != (directory.st_dev, directory.st_ino, directory.st_uid, directory.st_mode):
                raise ValueError("Private configuration parent changed during preflight")
        finally:
            os.close(descriptor)
    finally:
        os.close(parent)
    document = json.loads(contents, object_pairs_hook=unique_fields)
    if (not isinstance(document, dict) or set(document) != {"hostname", "login", "socket"}
            or not all(isinstance(value, str) for value in document.values())):
        raise ValueError("Private configuration requires exactly hostname, login and socket strings")
    spec = importlib.util.spec_from_file_location("launcher_private_policy", Path(__file__).with_name("make-private-serve-config.py"))
    policy = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(policy)
    policy.make_policy(document["hostname"], document["login"])
    socket = Path(document["socket"])
    if (not socket.is_absolute() or ".." in socket.parts or len(os.fsencode(socket)) > 103
            or socket.parent.resolve(strict=True) != socket.parent or os.path.lexists(socket)):
        raise ValueError("Private socket must be a new canonical absolute short path")
    private_directory(socket.parent)
    return {"sha256": hashlib.sha256(contents).hexdigest(), "size": len(contents),
            "device": after.st_dev, "inode": after.st_ino}


def native_helper_identity(path: Path) -> dict:
    actual = regular(path, executable=True)
    if actual != path:
        raise ValueError("Native helper must be canonical")
    outside_source(actual)
    metadata = actual.stat()
    parent_descriptor = directory_fd(actual.parent)
    try:
        parent = os.fstat(parent_descriptor)
        if (not stat.S_ISDIR(parent.st_mode) or parent.st_uid not in {0, os.getuid()}
                or parent.st_mode & 0o022 or metadata.st_size > 16 * 1024 * 1024):
            raise ValueError("Native helper parent/size is unsafe")
    finally:
        os.close(parent_descriptor)
    result = executable_identity(actual, maximum_size=16 * 1024 * 1024)
    current_parent = actual.parent.lstat()
    if (current_parent.st_dev, current_parent.st_ino, current_parent.st_uid, current_parent.st_mode) != (parent.st_dev, parent.st_ino, parent.st_uid, parent.st_mode):
        raise ValueError("Native helper parent changed during preflight")
    result.update(parent_device=parent.st_dev, parent_inode=parent.st_ino)
    return result


def frontend_tree(path: Path) -> Path:
    """Validate the complete served tree using no-follow directory descriptors."""
    if not path.is_absolute() or ".." in path.parts or path.resolve(strict=True) != path:
        raise ValueError("Built frontend tree must be canonical without symlinks")
    owner = os.getuid()
    identity = lambda value: (value.st_dev, value.st_ino, value.st_uid, value.st_mode, value.st_nlink, value.st_mtime_ns, value.st_ctime_ns)
    def walk(descriptor):
        before = os.fstat(descriptor)
        if not stat.S_ISDIR(before.st_mode) or before.st_uid != owner or before.st_mode & 0o022:
            raise ValueError("Frontend directories must be owned and not writable by others")
        names = os.listdir(descriptor)
        for name in names:
            original = os.stat(name, dir_fd=descriptor, follow_symlinks=False)
            if stat.S_ISDIR(original.st_mode):
                child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor)
                try:
                    if identity(original) != identity(os.fstat(child)):
                        raise ValueError("Frontend directory changed during validation")
                    walk(child)
                finally:
                    os.close(child)
            elif stat.S_ISREG(original.st_mode):
                child = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=descriptor)
                try:
                    opened = os.fstat(child)
                    if (identity(original) != identity(opened) or opened.st_uid != owner
                            or opened.st_nlink != 1 or opened.st_mode & (0o022 | stat.S_ISUID | stat.S_ISGID)):
                        raise ValueError("Frontend files must be owned single-link nonwritable-by-others regular files")
                finally:
                    os.close(child)
            else:
                raise ValueError("Frontend tree cannot contain symlinks or special files")
            if identity(original) != identity(os.stat(name, dir_fd=descriptor, follow_symlinks=False)):
                raise ValueError("Frontend tree changed during validation")
        if set(os.listdir(descriptor)) != set(names) or identity(before) != identity(os.fstat(descriptor)):
            raise ValueError("Frontend directory changed during validation")
    descriptor = directory_fd(path)
    try:
        before = os.fstat(descriptor)
        walk(descriptor)
        index = os.stat("index.html", dir_fd=descriptor, follow_symlinks=False)
        if not stat.S_ISREG(index.st_mode):
            raise ValueError("Built frontend requires a regular index.html")
        if identity(before) != identity(path.lstat()) or path.resolve(strict=True) != path:
            raise ValueError("Frontend root changed during validation")
    finally:
        os.close(descriptor)
    return path


def configuration(binary: Path, books_root: Path, assets: Path | None = None, private_config: Path | None = None, native_helper: Path | None = None) -> dict:
    if assets is None and (private_config is not None or native_helper is not None):
        raise ValueError("--private-config and --native-helper require --assets")
    binary = regular(binary, executable=True)
    executable_identity(binary)
    books_root = private_directory(books_root)
    validate_log(books_root / "selection.stdout.log")
    validate_log(books_root / "selection.stderr.log")
    arguments = [str(binary), "books", str(books_root)]
    if assets is not None:
        assets = frontend_tree(assets)
        arguments = [str(binary), "serve-chooser", str(books_root), str(assets)]
        if private_config is not None:
            private_configuration_identity(private_config)
            arguments[1] = "serve-private-chooser"
            arguments.append(str(private_config))
        if native_helper is not None:
            native_helper_identity(native_helper)
            arguments[1] = "serve-private-native-chooser" if private_config is not None else "serve-native-chooser"
            arguments.append(str(native_helper))
    return {
        "Label": LABEL,
        "Disabled": True,
        "ProgramArguments": arguments,
        "RunAtLoad": True,
        "KeepAlive": False,
        "ThrottleInterval": 30,
        "ProcessType": "Interactive",
        "Umask": 0o077,
        "StandardOutPath": str(books_root / "selection.stdout.log"),
        "StandardErrorPath": str(books_root / "selection.stderr.log"),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--books-root", required=True, type=Path)
    parser.add_argument("--assets", type=Path, help="Built frontend directory; starts an unselected loopback session")
    parser.add_argument("--private-config", type=Path, help="Existing private runtime config; requires --assets and does not configure Tailscale")
    parser.add_argument("--native-helper", type=Path, help="Existing trusted native helper; requires --assets and never opens a panel here")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--preflight", action="store_true", help="Validate and print readiness JSON without creating any files")
    args = parser.parse_args()
    try:
        value = configuration(args.binary, args.books_root, args.assets, args.private_config, args.native_helper)
        output, parent_descriptor = output_parent(args.output)
        try:
            identity = executable_identity(args.binary)
            readiness = {"label": LABEL, "mode": value["ProgramArguments"][1],
                         "disabled": True, "installed": False, "started": False,
                         "executable_identity": identity}
            if args.private_config is not None:
                readiness["private_config_identity"] = private_configuration_identity(args.private_config)
            if args.native_helper is not None:
                readiness["native_helper_identity"] = native_helper_identity(args.native_helper)
            if args.preflight:
                print(json.dumps(readiness, sort_keys=True))
                return
            payload = plistlib.dumps(value)
            descriptor = os.open(output.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                                 0o600, dir_fd=parent_descriptor)
            with os.fdopen(descriptor, "wb") as prepared:
                prepared.write(payload)
                prepared.flush()
                os.fsync(prepared.fileno())
            os.fsync(parent_descriptor)
        finally:
            os.close(parent_descriptor)
    except (ValueError, OSError) as error:
        parser.error(str(error))
    print(f"Prepared disabled plan {output}; nothing was installed or started.")



if __name__ == "__main__":
    main()
