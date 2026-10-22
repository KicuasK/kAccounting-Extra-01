#!/usr/bin/env python3
"""Prepare a disposable launchd acceptance plan; --run explicitly executes only that plan.

Never installs persistent login files, opens a book, or changes another job.
The selected executable must be the reviewed fake-key login_acceptance example.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import re
import socket
import stat
import subprocess
import tempfile
import time
import urllib.request

SPEC = importlib.util.spec_from_file_location("launcher_preparation", Path(__file__).with_name("make-launch-agent.py"))
PREPARATION = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREPARATION)
PREFIX = "local.kAccounting-Extra-01.acceptance."
LAUNCHCTL = "/bin/launchctl"
HOST = "127.0.0.1"
PORT = 3000


def write_new(path: Path, contents: bytes) -> None:
    parent = PREPARATION.directory_fd(path.parent)
    try:
        metadata = os.fstat(parent)
        if metadata.st_uid != os.getuid() or stat.S_IMODE(metadata.st_mode) != 0o700:
            raise ValueError("Acceptance output parent must remain private and owned")
        descriptor = os.open(path.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                             0o600, dir_fd=parent)
        with os.fdopen(descriptor, "wb") as output:
            output.write(contents)
            output.flush()
            os.fsync(output.fileno())
        os.fsync(parent)
    finally:
        os.close(parent)


def expected_job(root: Path, binary: Path, assets: Path, label: str) -> dict:
    return {"Label": label, "ProgramArguments": [str(binary), str(root / "served-assets"), str(root / "books"), label.removeprefix(PREFIX)],
            "RunAtLoad": True, "KeepAlive": False, "ProcessType": "Interactive",
            "Umask": 0o077, "StandardOutPath": str(root / "stdout.log"),
            "StandardErrorPath": str(root / "stderr.log")}


def prepare(binary: Path, assets: Path, parent: Path | None = None) -> Path:
    binary = PREPARATION.regular(binary, executable=True)
    if binary.name != "login_acceptance":
        raise ValueError("Select the reviewed login_acceptance fake-key example executable")
    identity = PREPARATION.executable_identity(binary)
    assets = PREPARATION.regular(assets / "index.html").parent
    parent = PREPARATION.private_directory(parent or Path(tempfile.gettempdir()).resolve(strict=True))
    nonce = os.urandom(16).hex()
    root = parent / ("ka-login-acceptance-" + nonce)
    root.mkdir(mode=0o700)
    (root / "books").mkdir(mode=0o700)
    served = root / "served-assets"
    served.mkdir(mode=0o700)
    # Copy only bounded regular built frontend files, never follow asset symlinks.
    for source in assets.rglob("*"):
        if source.is_symlink():
            raise ValueError("Built asset tree must not contain symlinks")
        relative = source.relative_to(assets)
        destination = served / relative
        if source.is_dir():
            destination.mkdir(mode=0o700)
        elif source.is_file() and source.stat().st_size <= 8 * 1024 * 1024:
            original = PREPARATION.regular(source)
            contents = original.read_bytes()
            if relative == Path("index.html"):
                contents = ("<!-- kAccounting-login-acceptance:" + nonce + " -->\n").encode() + contents
            write_new(destination, contents)
        else:
            raise ValueError("Built assets must be bounded regular files")
    for name in ["stdout.log", "stderr.log"]:
        write_new(root / name, b"")
    label = PREFIX + nonce
    job = expected_job(root, binary, assets, label)
    payload = plistlib.dumps(job)
    write_new(root / "job.plist", payload)
    plan = {"format": "kAccounting-Extra-01.login-acceptance", "version": 1,
            "root": str(root), "label": label, "uid": os.getuid(), "binary": str(binary),
            "assets": str(assets), "executable_identity": identity,
            "plist_sha256": hashlib.sha256(payload).hexdigest(),
            "index_sha256": hashlib.sha256((served / "index.html").read_bytes()).hexdigest()}
    write_new(root / "plan.json", json.dumps(plan, sort_keys=True).encode())
    return root / "plan.json"


def private_file(path: Path, limit: int) -> bytes:
    parent = PREPARATION.directory_fd(path.parent)
    try:
        descriptor = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
        try:
            metadata = os.fstat(descriptor)
            if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid()
                    or metadata.st_nlink != 1 or metadata.st_mode & 0o077 or metadata.st_size > limit):
                raise ValueError("Acceptance files must be bounded private owned single-link regular files")
            chunks = []
            remaining = limit + 1
            while remaining:
                chunk = os.read(descriptor, min(remaining, 65536))
                if not chunk:
                    break
                chunks.append(chunk)
                remaining -= len(chunk)
            contents = b"".join(chunks)
            after = os.fstat(descriptor)
            named = os.stat(path.name, dir_fd=parent, follow_symlinks=False)
            identity = lambda value: (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns)
            if len(contents) > limit or identity(metadata) != identity(after) or identity(after) != identity(named):
                raise ValueError("Acceptance file changed while being verified")
            return contents
        finally:
            os.close(descriptor)
    finally:
        os.close(parent)


def load_plan(path: Path) -> tuple[dict, dict]:
    root = PREPARATION.private_directory(path.parent)
    if path.name != "plan.json":
        raise ValueError("Select the exact prepared plan.json")
    plan = json.loads(private_file(root / "plan.json", 16384))
    expected_fields = {"format", "version", "root", "label", "uid", "binary", "assets", "executable_identity", "plist_sha256", "index_sha256"}
    if not isinstance(plan, dict) or set(plan) != expected_fields or plan["format"] != "kAccounting-Extra-01.login-acceptance" or plan["version"] != 1:
        raise ValueError("Unknown acceptance plan")
    if (any(not isinstance(plan[field], str) for field in ["root", "label", "binary", "assets", "plist_sha256", "index_sha256"])
            or type(plan["uid"]) is not int or type(plan["version"]) is not int
            or not isinstance(plan["executable_identity"], dict)):
        raise ValueError("Malformed acceptance field types")
    if plan["uid"] != os.getuid() or plan["root"] != str(root):
        raise ValueError("Acceptance plan belongs to another user or directory")
    match = re.fullmatch(re.escape(PREFIX) + r"([0-9a-f]{32})", plan["label"])
    if not match or root.name != "ka-login-acceptance-" + match.group(1):
        raise ValueError("Acceptance label is not bound to its unique fixture")
    if set(item.name for item in root.iterdir()) != {"books", "served-assets", "stdout.log", "stderr.log", "job.plist", "plan.json"}:
        raise ValueError("Unexpected acceptance directory contents; preserve for inspection")
    books = PREPARATION.private_directory(root / "books")
    if any(books.iterdir()):
        raise ValueError("Acceptance requires an empty disposable collection")
    binary = Path(plan["binary"])
    if binary.name != "login_acceptance" or PREPARATION.executable_identity(binary) != plan["executable_identity"]:
        raise ValueError("Acceptance executable changed since review")
    assets = PREPARATION.regular(Path(plan["assets"]) / "index.html").parent
    index = private_file(root / "served-assets" / "index.html", 8 * 1024 * 1024 + 200)
    marker = ("<!-- kAccounting-login-acceptance:" + match.group(1) + " -->\n").encode()
    if not index.startswith(marker) or hashlib.sha256(index).hexdigest() != plan["index_sha256"]:
        raise ValueError("Owned frontend identity changed since preparation")
    job = expected_job(root, binary, assets, plan["label"])
    contents = private_file(root / "job.plist", 16384)
    if hashlib.sha256(contents).hexdigest() != plan["plist_sha256"] or plistlib.loads(contents) != job:
        raise ValueError("Acceptance job changed since preparation")
    for log in ["stdout.log", "stderr.log"]:
        PREPARATION.validate_log(root / log)
    return plan, job


def launch(arguments: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run([LAUNCHCTL, *arguments], capture_output=True, text=True, timeout=10, check=False)


def port_available() -> bool:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        try:
            probe.bind((HOST, PORT))
            return True
        except OSError:
            return False


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


def local_opener():
    return urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())


def read_json(path: str) -> dict:
    opener = local_opener()
    with opener.open(f"http://{HOST}:{PORT}{path}", timeout=1) as response:
        if response.status != 200:
            raise ValueError("Unexpected acceptance HTTP status")
        payload = response.read(65537)
        if len(payload) > 65536:
            raise ValueError("Acceptance HTTP response exceeds limit")
        return json.loads(payload)


def owns_job(output: str, plan: dict) -> bool:
    expected = expected_job(Path(plan["root"]), Path(plan["binary"]), Path(plan["assets"]), plan["label"])
    lines = [line.strip() for line in output.splitlines()]
    target = f"gui/{plan['uid']}/{plan['label']} = {{"
    if not lines or lines[0] != target:
        return False
    if "path = " + str(Path(plan["root"]) / "job.plist") not in lines:
        return False
    if "program = " + plan["binary"] not in lines:
        return False
    try:
        start = lines.index("arguments = {") + 1
        end = lines.index("}", start)
    except ValueError:
        return False
    return lines[start:end] == expected["ProgramArguments"]


def verify_http_owner(plan: dict) -> None:
    token = plan["label"][len(PREFIX):]
    opener = local_opener()
    with opener.open(f"http://{HOST}:{PORT}/", timeout=1) as response:
        marker = ("<!-- kAccounting-login-acceptance:" + token + " -->\n").encode()
        if response.status != 200 or response.read(len(marker)) != marker:
            raise ValueError("Loopback server does not match this acceptance fixture")


def execute(path: Path, timeout: float = 15) -> dict:
    if not 1 <= timeout <= 60:
        raise ValueError("Acceptance timeout must be between 1 and 60 seconds")
    plan, _ = load_plan(path)
    target = f"gui/{plan['uid']}/{plan['label']}"
    domain = f"gui/{plan['uid']}"
    if not port_available():
        raise ValueError("Loopback port 3000 is already occupied; existing server was not touched")
    # Only an absent exact test target may be created. No broad unload or domain operations.
    collision = launch(["print", target])
    if collision.returncode == 0:
        raise ValueError("Unique acceptance job already exists; it was not changed")
    attempted = False
    verified = False
    result = None
    try:
        # Recheck every artifact immediately before the authorized mutation.
        load_plan(path)
        attempted = True
        started = launch(["bootstrap", domain, str(path.parent / "job.plist")])
        if started.returncode != 0:
            raise RuntimeError("Temporary acceptance bootstrap failed; inspect the retained fixture")
        deadline = time.monotonic() + timeout
        last_error = None
        while time.monotonic() < deadline:
            state = launch(["print", target])
            if state.returncode != 0:
                raise RuntimeError("Temporary acceptance job disappeared before HTTP verification")
            try:
                if not owns_job(state.stdout, plan):
                    raise RuntimeError("Temporary job identity does not match the reviewed fixture")
                verify_http_owner(plan)
                session = read_json("/api/session")
                books = read_json("/api/session/books")
                if session != {"book_id": None, "selection_required": True} or books != {"books": []}:
                    raise ValueError("Acceptance server did not start as an empty unselected session")
                verify_http_owner(plan)
                verified = True
                result = {"passed": True, "label": plan["label"], "selected_book": False,
                          "collection_empty": True, "persistent_install": False}
                break
            except (OSError, ValueError) as error:
                last_error = error
                time.sleep(0.1)
        if not verified:
            raise RuntimeError("Temporary acceptance HTTP verification timed out") from last_error
    finally:
        if attempted:
            identity = launch(["print", target])
            if identity.returncode == 0:
                if not owns_job(identity.stdout, plan):
                    raise RuntimeError("Cleanup refused: exact job ownership is unverified; inspect the retained test label")
                stopped = launch(["bootout", target])
                remaining = launch(["print", target])
                if remaining.returncode == 0:
                    raise RuntimeError("Own temporary job could not be removed; inspect exact retained test label")
                if stopped.returncode not in [0, 3, 113]:
                    raise RuntimeError("Own temporary job cleanup returned an unexpected result")
        if verified:
            deadline = time.monotonic() + 3
            while True:
                try:
                    lines = private_file(path.parent / "stdout.log", 65536).decode("utf-8").splitlines()
                except ValueError:
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(0.05)
                    continue
                events = []
                for line in lines:
                    try:
                        event = json.loads(line)
                    except ValueError:
                        continue
                    if isinstance(event, dict) and event.get("fixture_token") == plan["label"][len(PREFIX):]:
                        events.append(event)
                if any(event.get("phase") == "shutdown" for event in events):
                    break
                if time.monotonic() >= deadline:
                    raise RuntimeError("Own job stopped without a verified fake-key shutdown receipt")
                time.sleep(0.05)
            if {event.get("phase") for event in events} != {"startup", "shutdown"} or any(event.get("key_calls") != 0 for event in events):
                raise RuntimeError("Fake-key acceptance receipt did not prove zero calls")
            result["fake_key_calls"] = 0
            result["own_job_removed"] = True
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--assets", type=Path)
    parser.add_argument("--parent", type=Path)
    parser.add_argument("--plan", type=Path)
    parser.add_argument("--run", action="store_true", help="Execute only a reviewed plan after explicit operational approval")
    args = parser.parse_args()
    try:
        if args.run:
            if args.plan is None or any([args.binary, args.assets, args.parent]):
                parser.error("--run requires only --plan pointing to a previously reviewed plan.json")
            print(json.dumps(execute(args.plan), sort_keys=True))
        else:
            if args.plan is not None or args.binary is None or args.assets is None:
                parser.error("Prepare with --binary and --assets; then review the generated plan before --run")
            path = prepare(args.binary, args.assets, args.parent)
            plan, job = load_plan(path)
            print(json.dumps({"plan": str(path), "label": plan["label"], "installed": False,
                              "started": False, "proposed_arguments": job["ProgramArguments"],
                              "proposed_run": [str(Path(__file__).resolve()), "--run", "--plan", str(path)]}, sort_keys=True))
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
