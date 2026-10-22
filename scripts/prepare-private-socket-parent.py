#!/usr/bin/env python3
"""Plan one new empty Tailscale socket directory; only --apply requests sudo."""
from __future__ import annotations
import argparse
import importlib.util
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile

SPEC = importlib.util.spec_from_file_location("private_preparation", Path(__file__).with_name("make-launch-agent.py"))
PREP = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREP)
PARENT = Path("/Library/Tailscale")


def identity(metadata):
    return metadata.st_dev, metadata.st_ino


def parent_identity():
    descriptor = PREP.directory_fd(PARENT)
    try:
        metadata = os.fstat(descriptor)
        if metadata.st_uid != 0 or stat.S_IMODE(metadata.st_mode) != 0o755:
            raise ValueError("/Library/Tailscale must be an existing root-owned canonical mode-755 directory")
        return identity(metadata)
    finally:
        os.close(descriptor)


def same_parent(expected):
    if parent_identity() != expected:
        raise ValueError("Tailscale parent identity changed; no further administrator action is allowed")


def new_directory(path, expected_owner, expected_identity=None):
    metadata = path.lstat()
    if (not stat.S_ISDIR(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode)
            or metadata.st_uid != expected_owner or stat.S_IMODE(metadata.st_mode) != 0o700
            or (expected_identity is not None and identity(metadata) != expected_identity)):
        raise ValueError("New directory identity, owner or permissions do not match; preserve it for inspection")
    return identity(metadata)


def plan(nonce, receipt):
    if os.getuid() == 0 or os.getuid() != os.geteuid():
        raise ValueError("Run this tool as your ordinary logged-in user, not through sudo")
    if len(nonce) != 32 or any(char not in "0123456789abcdef" for char in nonce):
        raise ValueError("Nonce must be exactly 32 lowercase hexadecimal characters")
    parent = parent_identity()
    path = PARENT / f"ka-{os.getuid()}-{nonce}"
    if os.path.lexists(path):
        raise ValueError("The exact destination already exists; it will never be adopted or changed")
    if not receipt.is_absolute() or receipt.suffix != '.json' or '..' in receipt.parts:
        raise ValueError("Receipt must be a new absolute .json file without traversal")
    PREP.private_directory(receipt.parent)
    if os.path.lexists(receipt):
        raise ValueError("Existing receipt is preserved; choose a new .json file")
    return {"format": "kAccounting-Extra-01.socket-parent-plan.v1", "directory": str(path),
            "receipt": str(receipt), "uid": os.getuid(), "gid": os.getgid(),
            "commands": [["/usr/bin/sudo", "/bin/mkdir", "-m", "700", str(path)],
                         ["/usr/bin/sudo", "/usr/sbin/chown", f"{os.getuid()}:{os.getgid()}", str(path)]]}, parent


def apply(value, expected_parent):
    if not sys.stdin.isatty() or not sys.stderr.isatty():
        raise ValueError("Administrator approval requires Terminal: run the companion .command file as your ordinary user")
    path = Path(value["directory"])
    receipt = Path(value["receipt"])
    receipt_parent = PREP.directory_fd(receipt.parent)
    descriptor = None
    outcome = dict(value, status="not_started", directory_created=False)
    try:
        metadata = os.fstat(receipt_parent)
        if metadata.st_uid != os.getuid() or stat.S_IMODE(metadata.st_mode) != 0o700:
            raise ValueError("Receipt parent changed or is not private")
        descriptor = os.open(receipt.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=receipt_parent)
        same_parent(expected_parent)
        if os.path.lexists(path):
            raise ValueError("Destination appeared after review; no administrator action was taken")
        first = subprocess.run(value["commands"][0], check=False)
        if first.returncode != 0:
            raise RuntimeError("Administrator mkdir was not completed; no chown was attempted")
        outcome["directory_created"] = True
        created = new_directory(path, 0)
        same_parent(expected_parent)
        new_directory(path, 0, created)
        second = subprocess.run(value["commands"][1], check=False)
        if second.returncode != 0:
            raise RuntimeError("Administrator chown was not completed; preserve the new directory for inspection")
        same_parent(expected_parent)
        new_directory(path, os.getuid(), created)
        with os.scandir(path) as entries:
            if next(entries, None) is not None:
                raise ValueError("New directory is not empty; preserve it for inspection")
        outcome["status"] = "new_empty_private_socket_parent_ready"
        return outcome
    except BaseException:
        outcome["status"] = "incomplete_preserve_for_inspection"
        raise
    finally:
        if descriptor is not None:
            payload = json.dumps(outcome, sort_keys=True).encode()
            pending = memoryview(payload)
            while pending:
                written = os.write(descriptor, pending)
                if written <= 0:
                    raise OSError("Receipt write made no progress; preserve the new directory for inspection")
                pending = pending[written:]
            os.fsync(descriptor)
            os.close(descriptor)
            os.fsync(receipt_parent)
        os.close(receipt_parent)


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--nonce")
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--interactive", action="store_true", help="Terminal-only plain-language confirmation, then apply")
    args = parser.parse_args()
    try:
        if args.interactive:
            if args.apply or args.nonce or args.receipt:
                parser.error("--interactive must be used by itself")
            if not sys.stdin.isatty() or not sys.stderr.isatty():
                raise ValueError("請在 Terminal 中開啟此 .command 工具；此處不會要求管理員權限。")
            nonce = os.urandom(16).hex()
            location = Path(tempfile.mkdtemp(prefix="ka-socket-parent-receipt-")).resolve()
            receipt = location / "receipt.json"
        else:
            if args.receipt is None or (args.apply and args.nonce is None):
                parser.error("Provide --receipt; --apply also requires the exact reviewed --nonce")
            nonce, receipt = args.nonce or os.urandom(16).hex(), args.receipt
        value, parent = plan(nonce, receipt)
        if args.interactive:
            print("只會建立一個全新的空白私人資料夾，供私人連線放置通訊檔案。")
            print("不會修改既有 Tailscale 資料、帳簿、金鑰、登入項目或網路設定。")
            print("新資料夾：" + value["directory"])
            print("結果紀錄：" + value["receipt"])
            answer = input("按 Return 才開始，接著 macOS 可能要求管理員密碼；輸入其他文字則取消：")
            if answer:
                print("已取消；未執行管理員命令。")
                return
        if args.apply or args.interactive:
            print(json.dumps(apply(value, parent), sort_keys=True))
        else:
            print(json.dumps(dict(value, applied=False, nonce=nonce, proposed_apply=["/usr/bin/python3", str(Path(__file__).resolve()), "--apply", "--nonce", nonce, "--receipt", str(receipt)]), sort_keys=True))
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
