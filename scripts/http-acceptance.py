#!/usr/bin/env python3
"""Smoke checks against an explicitly identified disposable acceptance server."""
import argparse
import importlib.util
import os
import stat
import hashlib
import json
import pathlib
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


SOURCE = pathlib.Path(__file__).resolve().parents[1]
BASE_ORIGIN = "http://127.0.0.1:3000"
PRIVATE_MODE = False


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


def open_request(request, timeout=5):
    url = request.full_url if isinstance(request, urllib.request.Request) else request
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme + "://" + parsed.netloc != BASE_ORIGIN or parsed.username or parsed.password:
        raise ValueError("Acceptance requests must remain on the configured fixed origin")
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    return opener.open(request, timeout=timeout)


def load_private_origin(path):
    path = pathlib.Path(path)
    if not path.is_absolute() or ".." in path.parts or path.resolve(strict=True) != path or path == SOURCE or SOURCE in path.parents:
        raise ValueError("Private configuration must be canonical, outside source and without symlinks")
    descriptor = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in path.parts[1:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor)
            os.close(descriptor)
            descriptor = child
        parent = os.fstat(descriptor)
        if parent.st_uid != os.getuid() or stat.S_IMODE(parent.st_mode) != 0o700:
            raise ValueError("Private configuration parent must be user-owned mode 700")
        file = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=descriptor)
        try:
            before = os.fstat(file)
            if not stat.S_ISREG(before.st_mode) or before.st_uid != os.getuid() or stat.S_IMODE(before.st_mode) != 0o600 or before.st_nlink != 1 or before.st_size > 16384:
                raise ValueError("Private configuration must be a user-owned single-link mode-600 file of at most 16 KiB")
            contents = os.read(file, 16385)
            after = os.fstat(file)
            named = os.stat(path.name, dir_fd=descriptor, follow_symlinks=False)
            identity = lambda item: (item.st_dev,item.st_ino,item.st_size,item.st_mtime_ns,item.st_ctime_ns)
            if len(contents) > 16384 or identity(before) != identity(after) or identity(after) != identity(named):
                raise ValueError("Private configuration changed during verification")
        finally:
            os.close(file)
    finally:
        os.close(descriptor)
    def unique_fields(pairs):
        value = {}
        for key,item in pairs:
            if key in value: raise ValueError("Duplicate private configuration field")
            value[key]=item
        return value
    document = json.loads(contents, object_pairs_hook=unique_fields)
    if not isinstance(document,dict) or set(document) != {"hostname","login","socket"} or not all(isinstance(value,str) for value in document.values()):
        raise ValueError("Expected the exact private runtime configuration fields")
    socket = pathlib.Path(document["socket"])
    if not socket.is_absolute() or ".." in socket.parts:
        raise ValueError("Private socket path must be absolute without traversal")
    spec = importlib.util.spec_from_file_location("acceptance_private_policy", SOURCE / "scripts/make-private-serve-config.py")
    policy = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(policy)
    return policy.make_policy(document["hostname"],document["login"])["origin"]


def require_fixture_marker(body, token):
    marker = ("<!-- kAccounting-acceptance:" + token + " -->\n").encode()
    if not body.startswith(marker):
        raise ValueError("Refuse all writes: server does not match the exact disposable fixture")


def capability_path(path, token):
    path = pathlib.Path(path)
    if (not path.is_absolute() or ".." in path.parts or path.name != "transfer-capabilities.json"
            or path.parent.name != "ka-http-acceptance-" + token or path.parent.resolve(strict=True) != path.parent
            or SOURCE == path.parent or SOURCE in path.parents):
        raise ValueError("Capabilities must belong to the exact outside-source disposable fixture")
    metadata = path.parent.stat()
    if metadata.st_uid != os.getuid() or stat.S_IMODE(metadata.st_mode) != 0o700:
        raise ValueError("Capability parent must remain private and owned")
    return path


def read_capabilities(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        metadata = os.fstat(descriptor)
        if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid()
                or stat.S_IMODE(metadata.st_mode) != 0o600 or metadata.st_nlink != 1 or metadata.st_size > 16384):
            raise ValueError("Capabilities must be a bounded private owned single-link file")
        contents = os.read(descriptor, 16385)
        after = os.fstat(descriptor)
        named = path.lstat()
        identity = lambda item: (item.st_dev,item.st_ino,item.st_size,item.st_mtime_ns,item.st_ctime_ns)
        if len(contents) > 16384 or identity(metadata) != identity(after) or identity(after) != identity(named):
            raise ValueError("Capability file changed during verification")
        return json.loads(contents)
    finally:
        os.close(descriptor)


def forbidden_status(headers, status):
    expected = {403,404} if PRIVATE_MODE and set(headers) == {"Host"} else {403}
    assert status in expected


def get(path, headers=None):
    request = urllib.request.Request(BASE_ORIGIN + path, headers=headers or {})
    try:
        with open_request(request, timeout=5) as response:
            return response.status, {name.lower(): value for name, value in response.headers.items()}, response.read()
    except urllib.error.HTTPError as error:
        return error.code, {name.lower(): value for name, value in error.headers.items()}, error.read()


def main():
    global BASE_ORIGIN, PRIVATE_MODE
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--fixture-token", required=True)
    parser.add_argument("--writes", action="store_true", help="Exercise draft/post/backup on the identified disposable book")
    parser.add_argument("--choose-book", action="store_true", help="Require unselected startup, then explicitly select the disposable book")
    parser.add_argument("--transfer-capabilities", help="Public capability file emitted by disposable --chooser-transfer example")
    parser.add_argument("--private-config", type=pathlib.Path, help="External private runtime configuration; derives fixed canonical HTTPS origin")
    args = parser.parse_args()
    if args.private_config:
        try:
            BASE_ORIGIN = load_private_origin(args.private_config)
        except (OSError, ValueError):
            parser.error("Private configuration is unsafe, unavailable or invalid; review the outside-source private file")
        PRIVATE_MODE = True
    else:
        BASE_ORIGIN = "http://127.0.0.1:3000"
        PRIVATE_MODE = False
    if len(args.fixture_token) != 32 or not all(c in "0123456789abcdef" for c in args.fixture_token):
        parser.error("Fixture token must be the exact 32 lowercase hexadecimal characters emitted by the disposable example")
    if args.transfer_capabilities and not args.choose_book:
        parser.error("--transfer-capabilities requires --choose-book")
    trusted_capability_path = capability_path(args.transfer_capabilities, args.fixture_token) if args.transfer_capabilities else None
    status, headers, body = get("/")
    assert status == 200 and b"kAccounting-Extra-01" in body
    require_fixture_marker(body, args.fixture_token)
    assert headers.get("cache-control") == "no-store"
    assert "frame-ancestors" in headers.get("content-security-policy", "")
    if args.choose_book:
        status, _, body = get("/api/session")
        assert status == 200 and json.loads(body)["selection_required"] is True
        assert get("/api/accounts")[0] == 409
        status, _, body = get("/api/session/books")
        assert status == 200
        books = json.loads(body)["books"]
        assert len(books) == 1 and books[0]["id"] == "book"
        request = urllib.request.Request(BASE_ORIGIN + "/api/session/selection",
            data=b'{"book_id":"book"}', method="POST",
            headers={"Content-Type": "application/json", "Origin": BASE_ORIGIN})
        with open_request(request, timeout=30) as response:
            assert response.status == 200
    status, _, body = get("/api/health")
    assert status == 200 and json.loads(body)["service"] == "kAccounting-Extra-01"
    status, _, body = get("/api/accounts/tree")
    assert status == 200
    accounts = [node["account"] for node in json.loads(body)]
    assert {account["name"] for account in accounts} == {
        "驗收現金 Cash " + args.fixture_token, "驗收權益 Equity",
    }, "This check requires the disposable invented-data acceptance server"
    status, _, body = get("/api/reports/trial-balance/2026-10-22")
    assert status == 200
    report = json.loads(body)
    assert report["total_debit_minor"] == "1000"
    assert report["total_credit_minor"] == "1000"
    for path in ["/", "/api/health", "/assets/missing.js"]:
        for invalid in [{"Host": "evil.example"}, {"Origin": "https://evil.example"}, {"Sec-Fetch-Site": "cross-site"}]:
            forbidden_status(invalid, get(path, invalid)[0])
    print("Actual HTTP acceptance passed: frontend, health, invented account tree, exact balanced report, Host/Origin/fetch-site rejection.")
    if args.writes:
        # This token is generated by the disposable example and is bound to its
        # invented chart. Refuse writes unless the exact fixture matched above.
        def post(path, payload):
            request = urllib.request.Request(BASE_ORIGIN + path,
                data=json.dumps(payload).encode(), method="POST",
                headers={"Content-Type": "application/json", "Origin": BASE_ORIGIN})
            with open_request(request, timeout=30) as response:
                return response.status, json.load(response)
        cash = next(a["id"] for a in accounts if a["element"] == "asset")
        equity = next(a["id"] for a in accounts if a["element"] == "equity")
        amount = "9007199254740993"
        status, draft = post("/api/journal/drafts", {"entry_date": "2026-10-22",
            "reference": "DISPOSABLE-HTTP-" + args.fixture_token,
            "explanation": "Invented HTTP write acceptance only", "lines": [
                {"account_id": cash, "side": "debit", "amount_minor": amount},
                {"account_id": equity, "side": "credit", "amount_minor": amount}]})
        assert status == 201
        # urllib streams an iterable as actual chunked HTTP; do not buffer the
        # full receipt or write a plaintext temporary file for this acceptance.
        chunk = b"Disposable attachment streaming acceptance only.\n" * 1024
        chunks = 64
        digest = hashlib.sha256()
        for _ in range(chunks):
            digest.update(chunk)
        query = urllib.parse.urlencode({"display_name": "驗收 receipt.txt",
            "source_kind": "regular_file", "expected_revision": draft["revision"]})
        request = urllib.request.Request(BASE_ORIGIN + "/api/journal/entries/" + draft["id"] + "/attachments?" + query,
            data=(chunk for _ in range(chunks)), method="POST",
            headers={"Content-Type": "application/octet-stream", "Origin": BASE_ORIGIN})
        with open_request(request, timeout=30) as response:
            assert response.status == 201
            attachment = json.load(response)
        assert attachment["sha256"] == digest.hexdigest()
        assert attachment["byte_length"] == str(len(chunk) * chunks)
        downloaded = hashlib.sha256()
        byte_count = 0
        with open_request(BASE_ORIGIN + "/api/attachments/" + attachment["id"] + "/content", timeout=30) as response:
            assert response.status == 200
            assert response.headers["Content-Length"] == attachment["byte_length"]
            assert response.headers["Content-Disposition"].startswith("attachment;")
            assert response.headers["Cache-Control"] == "no-store"
            while True:
                part = response.read(65536)
                if not part:
                    break
                byte_count += len(part)
                downloaded.update(part)
        assert byte_count == len(chunk) * chunks and downloaded.hexdigest() == digest.hexdigest()
        status, _, body = get("/api/journal/drafts/" + draft["id"])
        assert status == 200
        draft = json.loads(body)
        assert draft["revision"] == "2"
        status, posted = post("/api/journal/drafts/" + draft["id"] + "/post", {"expected_revision": draft["revision"]})
        assert status == 200 and posted["lines"][0]["amount_minor"] == amount
        status, _, body = get("/api/reports/trial-balance/2026-10-22")
        assert status == 200
        report = json.loads(body)
        assert report["total_debit_minor"] == report["total_credit_minor"] == str(int(amount) + 1000)
        status, backup = post("/api/backups", {})
        assert status == 201 and backup["verified"] is True
        assert backup["attachment_count"] == "1"
        print("Actual HTTP write acceptance passed: draft, chunked attachment upload, bounded download with exact hash, atomic posting, integer beyond JavaScript safe range, balanced report, verified encrypted backup including attachment.")
    if args.transfer_capabilities:
        assert args.choose_book, "Transfer fixture requires explicit chooser mode"
        selected_capability_path = trusted_capability_path
        deadline = time.monotonic() + 10
        while not selected_capability_path.exists():
            assert time.monotonic() < deadline, "trusted fixture capability registration timed out"
            time.sleep(0.02)
        while True:
            try:
                capabilities = read_capabilities(selected_capability_path)
                break
            except (FileNotFoundError, json.JSONDecodeError):
                if time.monotonic() >= deadline:
                    raise ValueError("Trusted fixture capability publication did not complete")
                time.sleep(0.02)
        assert capabilities.pop("fixture_token") == args.fixture_token
        import_destination = capabilities.pop("import_destination")
        request = urllib.request.Request(BASE_ORIGIN + "/api/storage/exports",
            data=json.dumps(capabilities).encode(), method="POST",
            headers={"Content-Type": "application/json", "Origin": BASE_ORIGIN})
        with open_request(request, timeout=30) as response:
            assert response.status == 202
            operation_id = json.load(response)["operation_id"]
        deadline = time.monotonic() + 30
        while True:
            status, _, body = get("/api/storage/operations/" + operation_id)
            assert status == 200
            operation = json.loads(body)
            if operation["state"] != "running":
                break
            assert time.monotonic() < deadline
            time.sleep(0.02)
        assert operation["state"] == "succeeded", operation
        with tempfile.TemporaryFile() as encrypted:
            with open_request(BASE_ORIGIN + "/api/storage/operations/" + operation_id + "/download", timeout=30) as response:
                assert response.status == 200
                assert response.headers["Content-Type"] == "application/octet-stream"
                assert response.headers["Cache-Control"] == "no-store"
                assert response.headers["Content-Disposition"] == 'attachment; filename="kAccounting-Extra-01.kagextra"'
                while True:
                    chunk = response.read(65536)
                    if not chunk:
                        break
                    encrypted.write(chunk)
            encrypted.seek(0)
            query = urllib.parse.urlencode({"request": capabilities["request"], "destination": import_destination})
            request = urllib.request.Request(BASE_ORIGIN + "/api/storage/container-imports?" + query,
                data=iter(lambda: encrypted.read(65536), b""), method="POST",
                headers={"Content-Type": "application/octet-stream", "Origin": BASE_ORIGIN})
            with open_request(request, timeout=30) as response:
                assert response.status == 201
                imported = json.load(response)
            assert imported["selection_required"] is True
            assert imported["attachment_count"] == ("1" if args.writes else "0")
        status, _, body = get("/api/session")
        assert status == 200 and json.loads(body)["book_id"] == "book", "import must not auto-select"
        request = urllib.request.Request(BASE_ORIGIN + "/api/session/selection", data=b'{"book_id":"imported"}', method="POST",
            headers={"Content-Type": "application/json", "Origin": BASE_ORIGIN})
        with open_request(request, timeout=30) as response:
            assert response.status == 200
        status, _, body = get("/api/reports/trial-balance/2026-10-22")
        assert status == 200 and json.loads(body) == report
        if args.writes:
            actual = hashlib.sha256()
            count = 0
            with open_request(BASE_ORIGIN + "/api/attachments/" + attachment["id"] + "/content", timeout=30) as response:
                assert response.status == 200
                while True:
                    chunk = response.read(65536)
                    if not chunk:
                        break
                    actual.update(chunk)
                    count += len(chunk)
            assert actual.hexdigest() == digest.hexdigest() and count == int(attachment["byte_length"])
        print("Actual TCP transfer acceptance passed: owned export job, bounded encrypted download, chunked container upload, full new-book import, unchanged source selection, explicit switch, exact report and attachment preservation.")
    if args.choose_book:
        request = urllib.request.Request(BASE_ORIGIN + "/api/session/selection",
            method="DELETE", headers={"Origin": BASE_ORIGIN})
        with open_request(request, timeout=30) as response:
            assert response.status == 200
        assert get("/api/accounts")[0] == 409
        status, _, body = get("/api/session")
        assert status == 200 and json.loads(body)["selection_required"] is True
        print("Actual HTTP session acceptance passed: unselected startup, explicit discovery/selection, writes on invented fixture, explicit close returning to selection.")


if __name__ == "__main__":
    main()
