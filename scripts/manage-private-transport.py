#!/usr/bin/env python3
"""Fixed-path root transport registration/control. Default is read-only planning."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import stat
import subprocess
import sys
import uuid

ROOT = Path('/Library/Application Support/kAccounting-Extra-01/private-transport')
LABEL = 'local.kAccounting-Extra-01.private-transport'
JOB = 'system/' + LABEL
DAEMONS = Path('/Library/LaunchDaemons')
PLIST = DAEMONS / (LABEL + '.plist')
ARGV = ['/usr/bin/python3', '-I', '-S', str(ROOT / 'bin/private-transport-guard.py')]
FORMAT = 'kAccounting-Extra-01.private-transport-control.v1'


def directory(path):
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for name in Path(path).parts[1:]:
            child = os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
            value = os.fstat(fd)
            if value.st_uid != 0 or value.st_mode & 0o022:
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
        raw = os.read(child, maximum + 1)
        after = os.fstat(child)
        named = os.stat(name, dir_fd=fd, follow_symlinks=False)
        if (len(raw) > maximum or (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns)
                != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns)
                or (after.st_dev, after.st_ino) != (named.st_dev, named.st_ino)):
            raise ValueError('Installed control file changed')
        return raw
    finally:
        os.close(child)


def plan_bytes(startup=False):
    return plistlib.dumps({'Label': LABEL, 'Disabled': not startup, 'UserName': 'root',
        'RunAtLoad': startup, 'KeepAlive': False, 'Umask': 0o077,
        'WorkingDirectory': str(ROOT), 'ProgramArguments': ARGV})


def refuse_version_pending(root):
    try:
        os.stat('version-pending.json', dir_fd=root, follow_symlinks=False)
    except FileNotFoundError:
        return
    raise ValueError('Transport version publication requires its own reconciliation')


def validate(expected_receipt):
    if not re.fullmatch('[0-9a-f]{64}', expected_receipt or ''):
        raise ValueError('Independently retained install receipt digest required')
    root = directory(ROOT)
    try:
        refuse_version_pending(root)
        metadata = os.fstat(root)
        if metadata.st_gid != 0 or stat.S_IMODE(metadata.st_mode) != 0o700:
            raise ValueError('Installation must remain private root:wheel')
        raw = read(root, 'install-receipt.json', 0o600, 65536)
        if hashlib.sha256(raw).hexdigest() != expected_receipt:
            raise ValueError('Installation receipt digest mismatch')
        receipt = json.loads(raw)
        if (receipt.get('format') != 'kAccounting-Extra-01.private-transport-install.v1'
                or receipt.get('destination') != str(ROOT) or receipt.get('installed') is not True):
            raise ValueError('Installation contract mismatch')
        bins = os.open('bin', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=root)
        try:
            value = os.fstat(bins)
            if value.st_uid != 0 or value.st_gid != 0 or stat.S_IMODE(value.st_mode) != 0o700:
                raise ValueError('Unsafe installed executable directory')
            for name in ('tailscaled', 'tailscale', 'private-transport-guard.py'):
                contents = read(bins, name, 0o700, 256 * 1024 * 1024 if name != 'private-transport-guard.py' else 65536)
                expected = receipt['guard_sha256'] if name == 'private-transport-guard.py' else receipt['transport_sha256'][name]
                if hashlib.sha256(contents).hexdigest() != expected:
                    raise ValueError('Installed executable changed')
        finally:
            os.close(bins)
        state = os.open('state', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=root)
        try:
            value = os.fstat(state)
            if value.st_uid != 0 or value.st_gid != 0 or stat.S_IMODE(value.st_mode) != 0o700:
                raise ValueError('Unsafe transport state directory')
        finally:
            os.close(state)
        if read(root, 'disabled-launchdaemon-review.plist', 0o600, 65536) != plan_bytes():
            raise ValueError('Disabled root plan mismatch')
    finally:
        os.close(root)
    return receipt


def command(arguments):
    return subprocess.run(['/bin/launchctl', *arguments], capture_output=True, text=True,
        timeout=15, check=False, env={'PATH': '/usr/bin:/bin:/usr/sbin:/sbin', 'HOME': '/var/root', 'TMPDIR': '/private/tmp'})


def observe():
    result = command(['print', JOB])
    if (result.returncode == 113 and result.stderr == 'Bad request.\nCould not find service \"' + LABEL + '\" in domain for system\n'):
        return 'absent'
    if result.returncode != 0 or len(result.stdout) > 1024 * 1024:
        raise ValueError('Job status unknown')
    # Deliberately narrow launchctl text grammar: unfamiliar output is refused.
    output = result.stdout
    if not output.startswith(JOB + ' = {'):
        raise ValueError('Unexpected loaded job label')
    paths = re.findall(r'^\s*path = (.+)$', output, re.M)
    programs = re.findall(r'^\s*program = (.+)$', output, re.M)
    blocks = re.findall(r'^\s*arguments = \{\n(.*?)^\s*\}', output, re.M | re.S)
    if paths != [str(PLIST)] or programs != [ARGV[0]] or len(blocks) != 1:
        raise ValueError('Loaded job path or program mismatch')
    arguments = [line.strip() for line in blocks[0].splitlines() if line.strip()]
    # launchctl's print arguments are one literal value per line (without indexes).
    if arguments != ARGV:
        raise ValueError('Loaded job argument mismatch')
    return 'loaded'


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


def registration_matches(expected_registered=None):
    parent = directory(DAEMONS)
    try:
        try:
            raw = read(parent, PLIST.name, 0o600, 65536)
        except FileNotFoundError:
            return False
        if expected_registered is not None and (not re.fullmatch('[0-9a-f]{64}', expected_registered)
                or hashlib.sha256(raw).hexdigest() != expected_registered):
            raise ValueError('Registered plist digest mismatch')
        if raw != plan_bytes() and (expected_registered is None or raw != plan_bytes(True)):
            raise ValueError('System daemon registration mismatch; startup mode requires pinned plist digest')
        return True
    finally:
        os.close(parent)


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


def finish(root, intent, reconciled=False, outcome_kind='confirmed', actual_state=None):
    name = 'control-outcome-' + intent['id'] + '.json'
    outcome = {**intent, 'outcome': outcome_kind, 'reconciled': reconciled, 'actual_state': actual_state}
    try:
        write(root, name, encoded(outcome))
    except FileExistsError:
        previous = json.loads(read(root, name, 0o600, 65536))
        if any(previous.get(key) != outcome[key] for key in ('format', 'id', 'action', 'receipt_sha256', 'job', 'outcome')):
            raise ValueError('Existing outcome differs')
    current = json.loads(read(root, 'control-pending.json', 0o600, 65536))
    if current != intent:
        raise ValueError('Pending operation changed before completion')
    os.unlink('control-pending.json', dir_fd=root)
    os.fsync(root)



def registered_bytes(expected):
    if not re.fullmatch('[0-9a-f]{64}', expected or ''):
        raise ValueError('Explicit registered plist digest required')
    parent = directory(DAEMONS)
    try:
        raw = read(parent, PLIST.name, 0o600, 65536)
        if hashlib.sha256(raw).hexdigest() != expected or raw not in (plan_bytes(), plan_bytes(True)):
            raise ValueError('Unknown or stale registered plist')
        return raw
    finally:
        os.close(parent)


def startup_operation(action, expected_receipt, expected_registered, apply):
    current = registered_bytes(expected_registered)
    target = plan_bytes(action == 'startup-enable')
    if not apply:
        return {'action': action, 'operation': 'plan', 'changed': False,
                'registered_sha256': expected_registered,
                'proposed_sha256': hashlib.sha256(target).hexdigest() if action != 'startup-reconcile' else None,
                'reload_performed': False, 'reboot_start_qualified': False}
    root = directory(ROOT)
    operation_lock = None
    try:
        operation_lock = lock(root)
        refuse_version_pending(root)
        validate(expected_receipt)
        current = registered_bytes(expected_registered)
        if action == 'startup-reconcile':
            intent = json.loads(read(root, 'control-pending.json', 0o600, 65536))
            if (intent.get('format') != FORMAT or intent.get('receipt_sha256') != expected_receipt
                    or intent.get('job') != JOB or intent.get('action') not in ('startup-enable', 'startup-disable')
                    or not re.fullmatch('[0-9a-f]{32}', intent.get('id', ''))):
                raise ValueError('Startup pending contract mismatch')
            old_hash = intent.get('registered_sha256')
            new_hash = intent.get('proposed_sha256')
            expected_target = plan_bytes(intent['action'] == 'startup-enable')
            if new_hash != hashlib.sha256(expected_target).hexdigest() or old_hash not in (
                    hashlib.sha256(plan_bytes()).hexdigest(), hashlib.sha256(plan_bytes(True)).hexdigest()):
                raise ValueError('Startup pending hashes do not identify known schemas')
            if expected_registered == new_hash:
                old = read(root, 'startup-backup-' + intent['id'] + '.plist', 0o600, 65536)
                if old not in (plan_bytes(), plan_bytes(True)) or hashlib.sha256(old).hexdigest() != old_hash:
                    raise ValueError('Startup backup mismatch')
                outcome = 'confirmed'
            elif expected_registered == old_hash:
                outcome = 'acknowledged-abort'
            else:
                raise ValueError('Registered plist matches neither side of pending change')
            finish(root, intent, True, outcome)
            return {'action': action, 'outcome': outcome, 'changed': False,
                    'registered_sha256': expected_registered, 'reload_performed': False, 'reboot_start_qualified': False}
        try:
            os.stat('control-pending.json', dir_fd=root, follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            raise ValueError('Pending operation requires reconciliation')
        if current == target:
            return {'action': action, 'outcome': 'already-configured', 'changed': False,
                    'registered_sha256': expected_registered, 'reload_performed': False, 'reboot_start_qualified': False}
        # Read-only observation refuses a loaded unrelated job; no launchctl mutation.
        observe()
        intent = {'format': FORMAT, 'id': uuid.uuid4().hex, 'action': action,
                  'receipt_sha256': expected_receipt, 'job': JOB, 'outcome': 'pending',
                  'registered_sha256': expected_registered, 'proposed_sha256': hashlib.sha256(target).hexdigest(),
                  'enable_override_may_have_changed': False}
        write(root, 'control-pending.json', encoded(intent))
        write(root, 'control-intent-' + intent['id'] + '.json', encoded(intent))
        write(root, 'startup-backup-' + intent['id'] + '.plist', current)
        parent = directory(DAEMONS)
        try:
            temp = PLIST.name + '.startup-' + intent['id']
            write(parent, temp, target)
            if registered_bytes(expected_registered) != current:
                raise ValueError('Registration changed before atomic replacement')
            validate(expected_receipt)
            os.replace(temp, PLIST.name, src_dir_fd=parent, dst_dir_fd=parent)
            os.fsync(parent)
        finally:
            os.close(parent)
        registered_bytes(intent['proposed_sha256'])
        finish(root, intent)
        return {'action': action, 'outcome': 'confirmed', 'changed': True,
                'registered_sha256': intent['proposed_sha256'], 'reload_performed': False,
                'loaded_job_not_reloaded': True, 'reboot_start_qualified': False}
    finally:
        if operation_lock is not None:
            os.close(operation_lock)
        os.close(root)

def operate(action, expected_receipt, apply=False, acknowledge_state=None, expected_registered=None):
    validate(expected_receipt)
    if acknowledge_state is not None and (action != 'reconcile' or acknowledge_state not in ('loaded', 'absent')):
        raise ValueError('State acknowledgment is restricted to reconciliation')
    if action not in ('register', 'start', 'stop', 'status', 'reconcile', 'startup-enable', 'startup-disable', 'startup-reconcile'):
        raise ValueError('Unknown action')
    if apply and (sys.platform != 'darwin' or os.getuid() != 0 or os.geteuid() != 0 or not sys.flags.isolated):
        raise ValueError('Apply requires isolated root Python on macOS')
    if action.startswith('startup-'):
        return startup_operation(action, expected_receipt, expected_registered, apply)
    if not apply:
        return {'action': action, 'operation': 'plan', 'started': False, 'changed': False,
                'job': JOB, 'registration': str(PLIST), 'update_supported': False}
    if action == 'status':
        return {'action': action, 'registered': registration_matches(expected_registered), 'job_state': observe(), 'changed': False}
    root = directory(ROOT)
    operation_lock = None
    try:
        operation_lock = lock(root)
        refuse_version_pending(root)
        validate(expected_receipt)
        if action == 'reconcile':
            pending = json.loads(read(root, 'control-pending.json', 0o600, 65536))
            if (pending.get('format') != FORMAT or pending.get('receipt_sha256') != expected_receipt
                    or pending.get('job') != JOB or pending.get('action') not in ('register', 'start', 'stop')
                    or not re.fullmatch('[0-9a-f]{32}', pending.get('id', ''))):
                raise ValueError('Pending operation does not match installation')
            registered, state = registration_matches(expected_registered), observe()
            if acknowledge_state is not None and state != acknowledge_state:
                raise ValueError('Observed state differs from operator expectation')
            complete = (pending['action'] == 'register' and registered
                        or pending['action'] == 'start' and registered and state == 'loaded'
                        or pending['action'] == 'stop' and state == 'absent')
            if not complete and acknowledge_state is not None:
                if state != acknowledge_state:
                    raise ValueError('Observed state differs from operator acknowledgment')
                finish(root, pending, True, 'acknowledged-abort', state)
                return {'action': action, 'outcome': 'acknowledged-abort', 'mutations_locked': False,
                        'job_state': state, 'enable_override_may_have_changed': pending.get('enable_override_may_have_changed', False)}
            if not complete:
                return {'action': action, 'outcome': 'unknown', 'mutations_locked': True, 'registered': registered, 'job_state': state}
            finish(root, pending, True, actual_state=state)
            return {'action': action, 'outcome': 'confirmed', 'mutations_locked': False,
                    'original_action': pending['action'], 'job_state': state}
        # Refuse unresolved operations before writing any further history.
        try:
            os.stat('control-pending.json', dir_fd=root, follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            raise ValueError('Pending operation requires reconcile')
        intent = {'format': FORMAT, 'id': uuid.uuid4().hex, 'action': action,
                  'receipt_sha256': expected_receipt, 'job': JOB, 'outcome': 'pending',
                  'enable_override_may_have_changed': action == 'start'}
        write(root, 'control-pending.json', encoded(intent))
        write(root, 'control-intent-' + intent['id'] + '.json', encoded(intent))
        # Any exception below leaves pending durable, locking later mutations.
        validate(expected_receipt)
        if action == 'register':
            if observe() != 'absent':
                raise ValueError('Registration requires absent exact job')
            parent = directory(DAEMONS)
            try:
                write(parent, PLIST.name, plan_bytes())
            finally:
                os.close(parent)
            if not registration_matches(expected_registered):
                raise ValueError('Registration outcome unknown')
        else:
            if not registration_matches(expected_registered):
                raise ValueError('Exact registration required')
            current = observe()
            commands = []
            if action == 'start':
                if current == 'absent':
                    commands.extend([['enable', JOB], ['bootstrap', 'system', str(PLIST)]])
                commands.append(['kickstart', JOB])
            elif current == 'loaded':
                commands.append(['bootout', JOB])
            for arguments in commands:
                if command(arguments).returncode != 0:
                    raise ValueError('Control outcome unknown; reconcile required')
            if observe() != ('loaded' if action == 'start' else 'absent'):
                raise ValueError('Control final state unknown')
        finish(root, intent)
        return {'action': action, 'outcome': 'confirmed', 'changed': True, 'qualified': False,
                'process_health_verified': False, 'reboot_start_qualified': False}
    finally:
        if operation_lock is not None:
            os.close(operation_lock)
        os.close(root)


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument('action', choices=['register', 'start', 'stop', 'status', 'reconcile', 'startup-enable', 'startup-disable', 'startup-reconcile'])
    parser.add_argument('--expected-install-receipt-sha256', required=True)
    parser.add_argument('--expected-registered-plist-sha256', help='Required for startup changes and management of startup-enabled registration')
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument('--plan', action='store_true')
    modes.add_argument('--apply', action='store_true')
    parser.add_argument('--expected-state', '--acknowledge-state', dest='acknowledge_state', choices=['loaded', 'absent'],
        help='Reconcile only: explicitly acknowledge a known aborted operation; does not revert enable overrides')
    args = parser.parse_args()
    if args.action == 'reconcile' and not args.acknowledge_state:
        parser.error('Reconcile requires --expected-state loaded|absent for explicit observed-state acknowledgment')
    print(json.dumps(operate(args.action, args.expected_install_receipt_sha256, args.apply, args.acknowledge_state, args.expected_registered_plist_sha256), sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, KeyError, TypeError, subprocess.SubprocessError):
        print('Transport control refused or outcome unknown; pending records, state and control files preserved.', file=sys.stderr)
        sys.exit(1)
