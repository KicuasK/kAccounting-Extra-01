#!/usr/bin/env python3
"""Prepare, but never install or start, the private macOS login launcher."""

import argparse
import os
from pathlib import Path
import plistlib
import stat


def regular(path: Path, executable: bool = False) -> Path:
    if not path.is_absolute() or path.is_symlink():
        raise ValueError("Use an absolute regular-file path without a symlink")
    actual = path.resolve(strict=True)
    if not actual.is_file() or (executable and not os.access(actual, os.X_OK)):
        raise ValueError("The selected binary or file is unavailable")
    return actual


def configuration(binary: Path, book: Path, assets: Path) -> dict:
    binary = regular(binary, executable=True)
    if not book.is_absolute() or book.is_symlink():
        raise ValueError("Use an absolute private book directory")
    book = book.resolve(strict=True)
    if not book.is_dir() or stat.S_IMODE(book.stat().st_mode) != 0o700:
        raise ValueError("Book directory must be private (mode 700)")
    regular(book / "book.sqlite3")
    regular(book / "device-key.json")
    assets = regular(assets / "index.html").parent
    return {
        "Label": "local.kAccounting-Extra-01",
        "ProgramArguments": [str(binary), "serve", str(book), str(assets)],
        "RunAtLoad": True,
        "KeepAlive": {"SuccessfulExit": False},
        "ThrottleInterval": 30,
        "ProcessType": "Interactive",
        "Umask": 0o077,
        "StandardOutPath": str(book / "runtime.stdout.log"),
        "StandardErrorPath": str(book / "runtime.stderr.log"),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--book", required=True, type=Path)
    parser.add_argument("--assets", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    payload = plistlib.dumps(configuration(args.binary, args.book, args.assets))
    if not args.output.is_absolute():
        parser.error("--output must be an absolute new file")
    # Exclusive creation protects an existing user's launcher; this command
    # neither writes LaunchAgents automatically nor executes launchctl.
    descriptor = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "wb") as output:
        output.write(payload)
        output.flush()
        os.fsync(output.fileno())
    print(f"Prepared {args.output}; nothing was installed or started.")


if __name__ == "__main__":
    main()
