#!/usr/bin/env python3
"""Disposable Unix provenance probe. Never invokes/configures Tailscale.

Default prints a plan only. --listen --run explicitly opens one fresh Unix
socket for at most 300 seconds. No financial files, SQL or TCP are involved.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import secrets
import socket
import stat
import sys
import time

MAX_HEADERS = 16 * 1024
SOURCE = Path(__file__).resolve().parents[1]


def private_config(path):
    value = Path(path)
    info = value.lstat()
    if (not value.is_absolute() or value.resolve() != value or SOURCE == value
            or SOURCE in value.parents or not stat.S_ISREG(info.st_mode)
            or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o600
            or info.st_size > 4096):
        raise ValueError('unsafe private probe configuration')
    descriptor = os.open(value, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        opened = os.fstat(descriptor)
        if (opened.st_dev, opened.st_ino) != (info.st_dev, info.st_ino):
            raise ValueError('private probe configuration changed')
        raw = os.read(descriptor, 4097)
    finally:
        os.close(descriptor)
    def unique(pairs):
        result = {}
        for key, item in pairs:
            if key in result:
                raise ValueError('duplicate configuration key')
            result[key] = item
        return result
    result = json.loads(raw, object_pairs_hook=unique)
    if (not isinstance(result, dict) or set(result) != {'hostname', 'login'} or not all(isinstance(v, str) for v in result.values())
            or not all(v.isascii() and v and '\r' not in v and '\n' not in v for v in result.values())):
        raise ValueError('invalid private probe configuration')
    return result


def peer_credentials(connection):
    if sys.platform != 'darwin':
        raise ValueError('probe requires Darwin getpeereid')
    libc = ctypes.CDLL('/usr/lib/libSystem.B.dylib', use_errno=True)
    function = libc.getpeereid
    function.argtypes = [ctypes.c_int, ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint)]
    function.restype = ctypes.c_int
    uid, gid = ctypes.c_uint(), ctypes.c_uint()
    if function(connection.fileno(), ctypes.byref(uid), ctypes.byref(gid)) != 0:
        raise OSError(ctypes.get_errno(), 'peer credentials unavailable')
    return uid.value, gid.value


def sanitized_headers(raw, expected=None):
    if len(raw) > MAX_HEADERS or b'\r\n\r\n' not in raw:
        raise ValueError('invalid bounded request headers')
    block, body = raw.split(b'\r\n\r\n', 1)
    lines = block.split(b'\r\n')
    request = lines[0].split(b' ')
    if len(request) != 3 or request[0] != b'GET' or request[2] != b'HTTP/1.1':
        raise ValueError('invalid probe request line')
    headers = {}
    for line in lines[1:]:
        if b':' not in line or line.startswith((b' ', b'\t')):
            raise ValueError('invalid probe header')
        name, value = line.split(b':', 1)
        if not name or any(c not in b"!#$%&'*+-.^_`|~0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ" for c in name):
            raise ValueError('invalid probe header name')
        key = name.decode('ascii').lower()
        headers.setdefault(key, []).append(value.strip(b' \t'))
    names = ['host', 'origin', 'tailscale-user-login', 'tailscale-funnel-request',
             'x-forwarded-host', 'x-forwarded-proto', 'x-forwarded-for']
    report = {'header_counts': {key: len(headers.get(key, [])) for key in names},
              'unix_host_localhost': headers.get('host') == [b'localhost'],
              'forwarded_proto_https': headers.get('x-forwarded-proto') == [b'https'],
              'funnel_marker_present': 'tailscale-funnel-request' in headers}
    if expected:
        report['expected_forwarded_host_matches'] = headers.get('x-forwarded-host') == [expected['hostname'].encode('ascii')]
        report['expected_login_matches'] = headers.get('tailscale-user-login') == [expected['login'].encode('ascii')]
    return request[1], report


def matches(path, original, kind):
    try:
        current = path.lstat()
        return (current.st_dev, current.st_ino) == (original.st_dev, original.st_ino) and kind(current.st_mode)
    except OSError:
        return False


def cleanup_owned(parent, parent_info, endpoint, socket_info):
    if not matches(parent, parent_info, stat.S_ISDIR):
        return False
    if socket_info is not None:
        if not matches(endpoint, socket_info, stat.S_ISSOCK):
            return False
        endpoint.unlink()
    try:
        parent.rmdir()  # Unknown contents are preserved; never recurse.
        return True
    except OSError:
        return False


def checked_parent_root(value):
    path = Path(value)
    metadata = path.lstat()
    if (not path.is_absolute() or path.resolve() != path
            or SOURCE == path or SOURCE in path.parents
            or not stat.S_ISDIR(metadata.st_mode)
            or metadata.st_uid != os.getuid()
            or stat.S_IMODE(metadata.st_mode) != 0o700):
        raise ValueError('probe parent must be an owned canonical private directory outside source')
    return path, metadata


def listen(timeout, expected=None, parent_root=None):
    nonce = secrets.token_hex(16)
    base = Path('/private/tmp')
    if parent_root is not None:
        base, original = checked_parent_root(parent_root)
        if not matches(base, original, stat.S_ISDIR):
            raise ValueError('probe parent changed')
    parent = base / ('ka-p-' + nonce[:16])
    if len(os.fsencode(parent / 'p.sock')) > 103:
        raise ValueError('probe socket path is too long')
    parent.mkdir(mode=0o700)
    parent_info = parent.lstat()
    endpoint = parent / 'p.sock'
    socket_info = None
    report = {'probe_nonce': nonce, 'financial_data_accessed': False, 'tcp_listener': False}
    server = None
    try:
        server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        server.bind(str(endpoint))
        endpoint.chmod(0o600)
        socket_info = endpoint.lstat()
        server.listen(1)
        server.settimeout(timeout)
        print(json.dumps({'ready': True, 'socket_path': str(endpoint), 'probe_path': '/probe-' + nonce}), flush=True)
        connection, _ = server.accept()
        with connection:
            connection.settimeout(5)
            uid, gid = peer_credentials(connection)
            report.update(peer_uid=uid, peer_gid=gid, root_peer=uid == 0)
            authorized = False
            if uid == 0:
                raw = bytearray()
                deadline = time.monotonic() + 5
                while b'\r\n\r\n' not in raw:
                    connection.settimeout(max(0.001, deadline - time.monotonic()))
                    chunk = connection.recv(min(4096, MAX_HEADERS + 1 - len(raw)))
                    if not chunk or len(raw) + len(chunk) > MAX_HEADERS:
                        raise ValueError('incomplete or oversized probe headers')
                    raw.extend(chunk)
                    if time.monotonic() >= deadline:
                        raise ValueError('probe header deadline exceeded')
                target, headers = sanitized_headers(bytes(raw), expected)
                report.update(headers)
                authorized = (target == ('/probe-' + nonce).encode('ascii')
                              and headers['unix_host_localhost'] and headers['forwarded_proto_https']
                              and not headers['funnel_marker_present']
                              and headers['header_counts']['tailscale-user-login'] == 1)
                if expected:
                    authorized = authorized and headers['expected_forwarded_host_matches'] and headers['expected_login_matches']
            report['authorized_probe_only'] = authorized
            payload = json.dumps({'probe_nonce': nonce} if authorized else {'error': 'denied'}).encode('ascii')
            status = b'200 OK' if authorized else b'403 Forbidden'
            connection.sendall(b'HTTP/1.1 ' + status + b'\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: ' + str(len(payload)).encode('ascii') + b'\r\n\r\n' + payload)
    except (OSError, ValueError):
        report['probe_incomplete'] = True
    finally:
        if server is not None:
            server.close()
        report['owned_socket_cleanup_complete'] = cleanup_owned(parent, parent_info, endpoint, socket_info)
        print(json.dumps(report, sort_keys=True), flush=True)
    return 0 if report.get('authorized_probe_only') else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--listen', action='store_true')
    parser.add_argument('--run', action='store_true')
    parser.add_argument('--timeout', type=int, default=300)
    parser.add_argument('--config')
    parser.add_argument('--parent-root', help='Explicit pre-approved owned private parent; never created or removed')
    args = parser.parse_args()
    if not 1 <= args.timeout <= 300:
        parser.error('timeout must be 1 through 300 seconds')
    if not args.listen and not args.run:
        print(json.dumps({'deployment_enabled': False, 'planned_parent': '/private/tmp/private-0700-nonce-directory', 'socket_mode': '0600', 'requires_explicit_flags': '--listen --run'}))
        return 0
    if not args.listen or not args.run:
        parser.error('listening requires both --listen and --run')
    expected = private_config(args.config) if args.config else None
    return listen(args.timeout, expected, args.parent_root)


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError):
        print('probe configuration or local setup refused', file=sys.stderr)
        sys.exit(1)
