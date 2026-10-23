#!/usr/bin/env python3
"""Stopped-only private transport upgrades; no service mutation or state migration."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import uuid

ROOT = Path('/Library/Application Support/kAccounting-Extra-01/private-transport')
CONTROL = Path('/private/var/run/kaccounting-private-transport')
FORMAT = 'kAccounting-Extra-01.private-transport-version.v1'
REVIEW_FORMAT = 'kAccounting-Extra-01.private-transport-version-review.v1'
NAMES = ('tailscale', 'tailscaled', 'private-transport-guard.py')
MAXIMUM = 256 * 1024 * 1024

def directory(path):
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        traversed = Path('/')
        for name in Path(path).parts[1:]:
            child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
            value = os.fstat(fd)
            traversed = traversed / name
            run_parent = (Path(path) == CONTROL and traversed == Path('/private/var/run')
                and value.st_uid == 0 and value.st_gid == 1 and stat.S_IMODE(value.st_mode) == 0o775)
            if value.st_uid != 0 or (value.st_mode & 0o022 and not run_parent):
                raise ValueError('Untrusted installation ancestor')
        return fd
    except BaseException:
        os.close(fd)
        raise


def read(fd, name, mode, maximum):
    child = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
    try:
        before = os.fstat(child)
        if (not stat.S_ISREG(before.st_mode) or before.st_uid != 0 or before.st_gid != 0
                or before.st_nlink != 1 or stat.S_IMODE(before.st_mode) != mode
                or not 0 < before.st_size <= maximum):
            raise ValueError('Unsafe installed control file')
        chunks = bytearray()
        while len(chunks) <= maximum:
            block = os.read(child, min(1024 * 1024, maximum + 1 - len(chunks)))
            if not block:
                break
            chunks.extend(block)
        raw = bytes(chunks)
        after = os.fstat(child)
        named = os.stat(name, dir_fd=fd, follow_symlinks=False)
        if (len(raw) > maximum or (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns)
                != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns)
                or (after.st_dev, after.st_ino) != (named.st_dev, named.st_ino)):
            raise ValueError('Installed control file changed')
        return raw
    finally:
        os.close(child)


def write(fd, name, raw, mode=0o600):
    child = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode, dir_fd=fd)
    try:
        os.fchmod(child, mode)
        os.fchown(child, 0, 0)
        with os.fdopen(os.dup(child), 'wb') as stream:
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
    finally:
        os.close(child)
    os.fsync(fd)


def encoded(value):
    return (json.dumps(value, sort_keys=True) + '\n').encode()


def lock(root):
    try:
        fd = os.open('control.lock', os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=root)
        os.fchmod(fd, 0o600)
        os.fchown(fd, 0, 0)
        os.fsync(root)
    except FileExistsError:
        fd = os.open('control.lock', os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=root)
    try:
        value = os.fstat(fd)
        if (not stat.S_ISREG(value.st_mode) or value.st_nlink != 1 or value.st_uid != 0
                or value.st_gid != 0 or stat.S_IMODE(value.st_mode) != 0o600):
            raise ValueError('Unsafe transport operation lock')
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return fd
    except BaseException:
        os.close(fd)
        raise


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('Duplicate JSON key')
        result[key] = value
    return result


def decode(raw):
    return json.loads(raw, object_pairs_hook=unique)


def private_root(fd):
    value = os.fstat(fd)
    if value.st_uid != 0 or value.st_gid != 0 or stat.S_IMODE(value.st_mode) != 0o700:
        raise ValueError('Unsafe installation root')


def private_child(parent, name):
    child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
    value = os.fstat(child)
    if value.st_uid != 0 or value.st_gid != 0 or stat.S_IMODE(value.st_mode) != 0o700:
        os.close(child)
        raise ValueError('Unsafe private directory')
    return child


def snapshot(fd, budget=None):
    """Bounded content and identity snapshot; never follow links or open sockets."""
    budget = [0, 0] if budget is None else budget
    metadata = os.fstat(fd)
    own = (metadata.st_dev, metadata.st_ino, metadata.st_mtime_ns, metadata.st_ctime_ns)
    names = sorted(os.listdir(fd))
    values = {}
    for name in names:
        budget[0] += 1
        if budget[0] > 10000:
            raise ValueError('State tree exceeds bound')
        before = os.stat(name, dir_fd=fd, follow_symlinks=False)
        identity = [before.st_dev, before.st_ino, before.st_mode, before.st_uid,
                    before.st_gid, before.st_size, before.st_mtime_ns, before.st_ctime_ns]
        if stat.S_ISDIR(before.st_mode):
            child = private_child(fd, name)
            try:
                values[name] = {'identity': identity, 'children': snapshot(child, budget)}
            finally:
                os.close(child)
        else:
            raw = read(fd, name, 0o600, MAXIMUM)
            budget[1] += len(raw)
            if budget[1] > MAXIMUM:
                raise ValueError('State bytes exceed bound')
            values[name] = {'identity': identity, 'sha256': digest(raw)}
        after = os.stat(name, dir_fd=fd, follow_symlinks=False)
        if identity != [after.st_dev, after.st_ino, after.st_mode, after.st_uid,
                        after.st_gid, after.st_size, after.st_mtime_ns, after.st_ctime_ns]:
            raise ValueError('State changed during snapshot')
    after = os.fstat(fd)
    if names != sorted(os.listdir(fd)) or own != (after.st_dev, after.st_ino, after.st_mtime_ns, after.st_ctime_ns):
        raise ValueError('State directory changed during snapshot')
    return {'directory_identity': own, 'entries': values}


def state_digest(root):
    fd = private_child(root, 'state')
    try:
        result = digest(encoded(snapshot(fd)))
        named = os.stat('state', dir_fd=root, follow_symlinks=False)
        opened = os.fstat(fd)
        if (named.st_dev, named.st_ino) != (opened.st_dev, opened.st_ino):
            raise ValueError('State directory name changed')
        return result
    finally:
        os.close(fd)


def absent():
    label = 'local.kAccounting-Extra-01.private-transport'
    result = subprocess.run(['/bin/launchctl', 'print', 'system/' + label],
        capture_output=True, text=True, timeout=15, check=False,
        env={'PATH': '/usr/bin:/bin:/usr/sbin:/sbin', 'HOME': '/var/root', 'TMPDIR': '/private/tmp'})
    if (result.returncode != 113 or result.stdout or
            result.stderr != 'Bad request.\nCould not find service "' + label + '" in domain for system\n'):
        raise ValueError('Exact transport job must be absent; stop and inspect first')
    try:
        control = directory(CONTROL)
    except FileNotFoundError:
        return
    try:
        private_root(control)
        if os.listdir(control):
            raise ValueError('Control artifacts preserved; operator inspection required')
    finally:
        os.close(control)


def installed(root, expected):
    if not re.fullmatch('[0-9a-f]{64}', expected or ''):
        raise ValueError('Pinned install receipt digest required')
    raw = read(root, 'install-receipt.json', 0o600, 65536)
    receipt = decode(raw)
    if (digest(raw) != expected or receipt.get('format') != 'kAccounting-Extra-01.private-transport-install.v1'
            or receipt.get('destination') != str(ROOT) or receipt.get('installed') is not True):
        raise ValueError('Installation receipt mismatch')
    bins = private_child(root, 'bin')
    try:
        pair = {name: read(bins, name, 0o700, MAXIMUM) for name in NAMES}
    finally:
        os.close(bins)
    for name in NAMES:
        expected_file = receipt['guard_sha256'] if name == NAMES[2] else receipt['transport_sha256'][name]
        if digest(pair[name]) != expected_file:
            raise ValueError('Installed executable differs')
    if b"UPGRADE_PROTOCOL = 'kAccounting-Extra-01.transport-version-guard.v1'\n" not in pair[NAMES[2]]:
        raise ValueError('Legacy guard lacks version publication protection; reinstall review required')
    return raw, receipt, pair


def input_file(path, mode, maximum):
    path = Path(path)
    if not path.is_absolute() or '..' in path.parts:
        raise ValueError('Absolute input path required')
    # Inputs are data only, never import or execute them. Walk every component nofollow.
    parent = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for component in path.parts[1:-1]:
            child = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
            os.close(parent)
            parent = child
            ancestor = os.fstat(parent)
            # Shared sticky temporary ancestors are allowed; the immediate staging
            # directory below must remain private to root or the sudo operator.
            if ancestor.st_uid not in (0, int(os.environ.get('SUDO_UID', '-1'))) or (ancestor.st_mode & 0o022 and not ancestor.st_mode & stat.S_ISVTX):
                raise ValueError('Untrusted source ancestor')
        ancestor = os.fstat(parent)
        if stat.S_IMODE(ancestor.st_mode) != 0o700:
            raise ValueError('Private staging directory required')
        fd = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
        try:
            before = os.fstat(fd)
            if (not stat.S_ISREG(before.st_mode) or before.st_nlink != 1
                    or before.st_uid not in (0, int(os.environ.get('SUDO_UID', '-1')))
                    or stat.S_IMODE(before.st_mode) != mode or not 0 < before.st_size <= maximum):
                raise ValueError('Unsafe reviewed input')
            raw = bytearray()
            while len(raw) <= maximum:
                block = os.read(fd, min(1024 * 1024, maximum + 1 - len(raw)))
                if not block:
                    break
                raw.extend(block)
            after = os.fstat(fd)
            named = os.stat(path.name, dir_fd=parent, follow_symlinks=False)
            fields = lambda item: (item.st_dev, item.st_ino, item.st_size, item.st_mtime_ns, item.st_ctime_ns)
            if fields(before) != fields(after) or fields(after) != fields(named) or len(raw) > maximum:
                raise ValueError('Reviewed input changed')
            return bytes(raw)
        finally:
            os.close(fd)
    finally:
        os.close(parent)


def reviewed(path, expected, old):
    raw = input_file(path, 0o600, 65536)
    if digest(raw) != expected:
        raise ValueError('Version review digest differs')
    review = decode(raw)
    provenance = review.get('provenance')
    if (review.get('format') != REVIEW_FORMAT or not isinstance(provenance, dict)
            or not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+', provenance.get('version', ''))
            or provenance.get('source_url') != 'https://github.com/tailscale/tailscale'
            or not re.fullmatch('[0-9a-f]{40}', provenance.get('source_revision', ''))
            or not isinstance(provenance.get('build_note'), str)
            or not 1 <= len(provenance['build_note']) <= 2000):
        raise ValueError('Explicit operator provenance assertion required')
    pair = {NAMES[2]: old[NAMES[2]]}
    for name in NAMES[:2]:
        entry = review['transport'][name]
        value = input_file(entry['path'], 0o700, MAXIMUM)
        if digest(value) != entry['sha256']:
            raise ValueError('Reviewed binary digest differs')
        # Only matching thin 64-bit Mach-O CPU type; do not execute untrusted binaries.
        if value[:4] != b'\xcf\xfa\xed\xfe' or len(value) < 32 or old[name][:4] != value[:4] or old[name][4:8] != value[4:8]:
            raise ValueError('Matching installed thin Mach-O architecture required')
        pair[name] = value
    return pair, provenance


def exists(fd, name):
    try:
        os.stat(name, dir_fd=fd, follow_symlinks=False)
        return True
    except FileNotFoundError:
        return False


def replace(root, name, raw):
    temporary = '.version-' + uuid.uuid4().hex
    write(root, temporary, raw)
    os.rename(temporary, name, src_dir_fd=root, dst_dir_fd=root)
    os.fsync(root)


def pair_matches(root, name, hashes):
    fd = private_child(root, name)
    try:
        if set(os.listdir(fd)) != set(NAMES):
            raise ValueError('Unexpected executable directory contents')
        guard = read(fd, NAMES[2], 0o700, 65536)
        if b"UPGRADE_PROTOCOL = 'kAccounting-Extra-01.transport-version-guard.v1'\n" not in guard:
            raise ValueError('Version archive guard protocol differs')
        return all(digest(read(fd, item, 0o700, MAXIMUM)) == hashes[item] for item in NAMES)
    finally:
        os.close(fd)


def eligible_state(root, pending):
    if state_digest(root) != pending['state_sha256']:
        raise ValueError('State changed; preserve journal for manual review')
    if pending['action'] == 'rollback' and exists(root, 'version-use-' + digest(bytes.fromhex(pending['before_receipt'])) + '.json'):
        raise ValueError('Rollback source was executed')


def complete(root, pending):
    """Only finish forward through a known journal; retain all old versions."""
    if pending.get('format') != FORMAT or not re.fullmatch('[0-9a-f]{32}', pending.get('id', '')):
        raise ValueError('Invalid version journal')
    ident = pending['id']
    if pending != decode(read(root, 'version-intent-' + ident + '.json', 0o600, 262144)) or pending.get('action') not in ('upgrade', 'rollback'):
        raise ValueError('Journal differs from immutable intent')
    oldname, newname = 'version-old-' + ident, 'version-new-' + ident
    before, after = bytes.fromhex(pending['before_receipt']), bytes.fromhex(pending['after_receipt'])
    for raw, hashes in ((before, pending['before_hashes']), (after, pending['after_hashes'])):
        receipt = decode(raw)
        if (receipt.get('format') != 'kAccounting-Extra-01.private-transport-install.v1'
                or receipt.get('destination') != str(ROOT) or receipt.get('installed') is not True
                or receipt.get('guard_sha256') != hashes.get(NAMES[2])
                or receipt.get('transport_sha256') != {name: hashes.get(name) for name in NAMES[:2]}):
            raise ValueError('Journal receipt contract mismatch')
    if pending['action'] == 'upgrade' and decode(after).get('previous_install_receipt_sha256') != digest(before):
        raise ValueError('Upgrade receipt chain differs')
    if pending['action'] == 'rollback' and decode(before).get('previous_install_receipt_sha256') != digest(after):
        raise ValueError('Rollback receipt chain differs')
    if pending['action'] == 'rollback' and exists(root, 'version-use-' + digest(before) + '.json'):
        raise ValueError('Rollback source was executed')
    if pending['before_hashes'][NAMES[2]] != pending['after_hashes'][NAMES[2]]:
        raise ValueError('Guard changes unsupported')
    eligible_state(root, pending)
    current = read(root, 'install-receipt.json', 0o600, 65536)
    if current not in (before, after):
        raise ValueError('Receipt is outside journal')
    if not exists(root, oldname):
        if not pair_matches(root, 'bin', pending['before_hashes']) or not pair_matches(root, newname, pending['after_hashes']):
            raise ValueError('Staged versions differ')
        os.rename('bin', oldname, src_dir_fd=root, dst_dir_fd=root)
        os.fsync(root)
    if not pair_matches(root, oldname, pending['before_hashes']):
        raise ValueError('Preserved version differs')
    if not exists(root, 'bin'):
        if not pair_matches(root, newname, pending['after_hashes']):
            raise ValueError('New staged version differs')
        os.rename(newname, 'bin', src_dir_fd=root, dst_dir_fd=root)
        os.fsync(root)
    if not pair_matches(root, 'bin', pending['after_hashes']):
        raise ValueError('Published version differs')
    eligible_state(root, pending)
    replace(root, 'install-receipt.json', after)
    outcome = {**pending, 'outcome': 'confirmed', 'started': False}
    name = 'version-outcome-' + ident + '.json'
    if exists(root, name):
        if decode(read(root, name, 0o600, 262144)) != outcome:
            raise ValueError('Outcome differs')
    else:
        write(root, name, encoded(outcome))
    if decode(read(root, 'version-pending.json', 0o600, 262144)) != pending:
        raise ValueError('Pending journal changed before completion')
    eligible_state(root, pending)
    os.unlink('version-pending.json', dir_fd=root)
    os.fsync(root)
    return {'outcome': 'confirmed', 'id': ident, 'action': pending['action'],
            'install_receipt_sha256': digest(after), 'started': False, 'state_restored': False}


def operate(action, expected, apply=False, review_path=None, review_sha=None, rollback_id=None):
    if action not in ('upgrade', 'rollback', 'reconcile'):
        raise ValueError('Unknown version action')
    root = directory(ROOT)
    operation_lock = None
    try:
        private_root(root)
        if action == 'reconcile':
            pending = decode(read(root, 'version-pending.json', 0o600, 262144))
            if expected not in (digest(bytes.fromhex(pending['before_receipt'])), digest(bytes.fromhex(pending['after_receipt']))):
                raise ValueError('Reconciliation receipt differs')
        else:
            oldraw, receipt, oldpair = installed(root, expected)
            if exists(root, 'version-pending.json') or exists(root, 'control-pending.json'):
                raise ValueError('Pending operation requires reconciliation')
            if action == 'upgrade':
                pair, provenance = reviewed(review_path, review_sha, oldpair)
                newreceipt = {**receipt, 'version_generation': uuid.uuid4().hex,
                    'previous_install_receipt_sha256': expected, 'version_review_sha256': review_sha,
                    'operator_provenance_assertion': provenance,
                    'transport_sha256': {name: digest(pair[name]) for name in NAMES[:2]}}
                newraw = encoded(newreceipt)
            else:
                if not re.fullmatch('[0-9a-f]{32}', rollback_id or ''):
                    raise ValueError('Pinned upgrade ID required')
                history = decode(read(root, 'version-outcome-' + rollback_id + '.json', 0o600, 262144))
                if history.get('action') != 'upgrade' or history.get('outcome') != 'confirmed' or digest(bytes.fromhex(history['after_receipt'])) != expected:
                    raise ValueError('Rollback must target immediately current completed upgrade')
                if digest(bytes.fromhex(history['before_receipt'])) != receipt.get('previous_install_receipt_sha256'):
                    raise ValueError('Rollback history receipt chain differs')
                if exists(root, 'version-use-' + expected + '.json'):
                    raise ValueError('New daemon execution attempted; binary rollback compatibility unqualified')
                if state_digest(root) != history['state_sha256']:
                    raise ValueError('Transport state changed; binary rollback refused')
                if not pair_matches(root, 'version-old-' + rollback_id, history['before_hashes']):
                    raise ValueError('Rollback archive differs')
                fd = private_child(root, 'version-old-' + rollback_id)
                try:
                    pair = {name: read(fd, name, 0o700, MAXIMUM) for name in NAMES}
                finally:
                    os.close(fd)
                if {name: digest(pair[name]) for name in NAMES} != history['before_hashes']:
                    raise ValueError('Preserved rollback bytes differ')
                newraw = bytes.fromhex(history['before_receipt'])
        if action != 'reconcile' and len(newraw) > 65536:
            raise ValueError('New receipt exceeds bound')
        if not apply:
            plan = {'action': action, 'operation': 'plan', 'changed': False, 'started': False,
                    'requires_absent_job': True, 'stop_status_checked': False, 'state_migration_supported': False}
            if action == 'reconcile':
                plan.update({'pending_id': pending['id'],
                    'current_install_receipt_sha256': expected,
                    'before_install_receipt_sha256': digest(bytes.fromhex(pending['before_receipt'])),
                    'target_install_receipt_sha256': digest(bytes.fromhex(pending['after_receipt']))})
            else:
                plan.update({'current_install_receipt_sha256': expected,
                    'target_install_receipt_sha256': digest(newraw),
                    'target_receipt_role': 'provisional-generation' if action == 'upgrade' else 'retained-exact-receipt',
                    'proposed_transport_sha256': {name: digest(pair[name]) for name in NAMES[:2]},
                    'review_sha256': review_sha if action == 'upgrade' else None,
                    'operator_provenance_assertion': provenance if action == 'upgrade' else None,
                    'rollback_of': rollback_id})
            return plan
        if sys.platform != 'darwin' or os.getuid() != 0 or os.geteuid() != 0 or not sys.flags.isolated:
            raise ValueError('Apply requires isolated root macOS Python')
        operation_lock = lock(root)
        if exists(root, 'control-pending.json'):
            raise ValueError('Control reconciliation required')
        absent()
        if action == 'reconcile':
            if pending != decode(read(root, 'version-pending.json', 0o600, 262144)):
                raise ValueError('Pending journal changed')
            return complete(root, pending)
        if exists(root, 'version-pending.json'):
            raise ValueError('Version reconciliation required')
        if installed(root, expected)[0] != oldraw:
            raise ValueError('Installation changed while waiting for lock')
        if action == 'rollback' and (exists(root, 'version-use-' + expected + '.json') or state_digest(root) != history['state_sha256']):
            raise ValueError('Rollback eligibility changed')
        ident = uuid.uuid4().hex
        name = 'version-new-' + ident
        os.mkdir(name, 0o700, dir_fd=root)
        fd = private_child(root, name)
        try:
            for item in NAMES:
                write(fd, item, pair[item], 0o700)
        finally:
            os.close(fd)
        pending = {'format': FORMAT, 'id': ident, 'action': action,
            'before_receipt': oldraw.hex(), 'after_receipt': newraw.hex(),
            'before_hashes': {name: digest(oldpair[name]) for name in NAMES},
            'after_hashes': {name: digest(pair[name]) for name in NAMES},
            'state_sha256': history['state_sha256'] if action == 'rollback' else state_digest(root), 'rollback_of': rollback_id}
        eligible_state(root, pending)
        if len(encoded(pending)) > 262144:
            raise ValueError('Journal exceeds reconciliation bound; staged bytes preserved')
        write(root, 'version-intent-' + ident + '.json', encoded(pending))
        write(root, 'version-pending.json', encoded(pending))
        absent()
        return complete(root, pending)
    finally:
        if operation_lock is not None:
            os.close(operation_lock)
        os.close(root)


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument('action', choices=['upgrade', 'rollback', 'reconcile'])
    parser.add_argument('--expected-install-receipt-sha256', required=True)
    parser.add_argument('--review')
    parser.add_argument('--expected-review-sha256')
    parser.add_argument('--upgrade-id')
    group = parser.add_mutually_exclusive_group()
    group.add_argument('--apply', action='store_true')
    group.add_argument('--plan', action='store_true')
    args = parser.parse_args()
    print(json.dumps(operate(args.action, args.expected_install_receipt_sha256, args.apply,
        args.review, args.expected_review_sha256, args.upgrade_id), sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, KeyError, TypeError, subprocess.SubprocessError):
        print('Transport version operation refused or interrupted; files and pending journal preserved.', file=sys.stderr)
        sys.exit(1)
