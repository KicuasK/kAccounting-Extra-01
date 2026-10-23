#!/usr/bin/env python3
"""Plan exact user-agent operations; --apply is required for any write or mutating launchctl call.

Never configures transport, opens books, removes sockets or replaces runtime trees.
Updates and rollback only replace an unloaded agent's plist; old bytes are retained.
"""
from __future__ import annotations
import argparse
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import re
import subprocess
import uuid

LABEL = 'local.kAccounting-Extra-01'

def module():
    spec = importlib.util.spec_from_file_location('deployment', Path(__file__).with_name('prepare-private-deployment.py'))
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value

def digest(raw):
    return hashlib.sha256(raw).hexdigest()

def read_private(path):
    dep = module()
    info, raw = dep.identity(path, 1024 * 1024)
    if info['mode'] != '0o600':
        raise ValueError('Agent and receipt files require mode 600')
    return raw

def launch_state(run, domain, expected=None):
    result = run(['/bin/launchctl', 'print', domain], capture_output=True, text=True, timeout=15)
    if result.returncode == 0:
        if expected is not None:
            fields = dict(line.strip().split(' = ', 1) for line in result.stdout.splitlines() if ' = ' in line)
            if fields.get('path') != str(expected[0]) or fields.get('program') != expected[1][0]:
                raise ValueError('Loaded job does not match pinned agent path and program')
            match = re.search(r'(?m)^\s*arguments = \{\n(.*?)^\s*\}', result.stdout, re.S)
            if match is None or [line.strip() for line in match.group(1).splitlines() if line.strip()] != expected[1]:
                raise ValueError('Loaded job arguments are ambiguous or differ from pinned agent')
        return 'loaded'
    # launchctl's documented missing-service code; other failures are unknown.
    if result.returncode == 113 and f'Could not find service "{LABEL}"' in result.stderr and f'domain for user gui: {os.getuid()}' in result.stderr:
        return 'absent'
    raise ValueError('Unknown launchctl outcome; inspect exact job before retrying')

def write_new(directory, name, raw):
    fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=directory)
    with os.fdopen(fd, 'wb') as stream:
        stream.write(raw)
        stream.flush()
        os.fsync(stream.fileno())
    os.fsync(directory)

def execute(action, home, review=None, expected_review=None, expected_installed=None,
            apply=False, run=subprocess.run, expected_state=None, start_at_login=False):
    dep = module()
    if start_at_login and action not in {'install', 'update', 'rollback'}:
        raise ValueError('--start-at-login is only valid for install/update/rollback')
    if apply and os.getuid() == 0:
        raise ValueError('Run user app lifecycle as the login user, never root')
    target = home / 'Library/LaunchAgents' / (LABEL + '.plist')
    domain = f'gui/{os.getuid()}/{LABEL}'
    report = {'action': action, 'target': str(target), 'domain': domain,
              'applied': False, 'transport_readiness': 'operator-required',
              'books_keys_runtime_trees': 'untouched'}
    report['unresolved_intents'] = [str(item) for item in target.parent.glob('.' + LABEL + '.intent-*.json')]
    if action == 'status':
        report['state'] = launch_state(run, domain)
        if target.exists():
            report['installed_sha256'] = digest(read_private(target))
        return report
    old = None
    if target.exists() or target.is_symlink():
        old_identity, old = dep.identity(target, 1024 * 1024)
        read_private(target)
        if not expected_installed or digest(old) != dep.sha(expected_installed):
            raise ValueError('Supply independently retained installed plist SHA256')
        parsed = plistlib.loads(old)
        if parsed.get('Label') != LABEL:
            raise ValueError('Unexpected installed label')
    elif action not in {'install', 'reconcile'}:
        raise ValueError('No installed app agent')
    if action == 'reconcile' and expected_state not in {'loaded', 'absent'}:
        raise ValueError('Reconcile requires explicit expected-state loaded or absent')
    if action == 'install' and old is not None:
        raise ValueError('Install preserves existing agent; use update')
    payload = None
    if action in {'install', 'update', 'rollback', 'start'}:
        if review is None or expected_review is None:
            raise ValueError('A pinned non-example deployment review is required')
        dep.revalidate_inputs(review, expected_review)
        prepared = plistlib.loads(read_private(Path(review) / 'disabled-user-launchagent.plist'))
        if prepared.get('Label') != LABEL:
            raise ValueError('Unexpected reviewed label')
        login_option = start_at_login
        if action == 'start':
            login_option = plistlib.loads(old).get('RunAtLoad')
            if type(login_option) is not bool:
                raise ValueError('Installed RunAtLoad must be an explicit boolean')
        prepared.update(Disabled=False, RunAtLoad=login_option)
        report['start_at_login'] = login_option
        report['login_reboot_qualified'] = False
        payload = plistlib.dumps(prepared)
        report['proposed_sha256'] = digest(payload)
        if action == 'rollback':
            retained = list(target.parent.glob('.' + LABEL + '.*.before-*.plist'))
            if not any(read_private(item) == payload for item in retained):
                raise ValueError('Rollback requires a retained prior exact plist; use update for new releases')
        if action == 'start' and payload != old:
            raise ValueError('Installed agent differs from selected review; update first')
    if not apply:
        report['phase'] = 'read-only-plan'
        return report
    # Secure directory walk rejects symlink ancestors. Never create Library or LaunchAgents.
    parent = dep.launcher.directory_fd(target.parent)
    try:
        metadata = os.fstat(parent)
        if metadata.st_uid != os.getuid() or metadata.st_mode & 0o022:
            raise ValueError('LaunchAgents must be user owned and not writable by others')
        lockname = '.' + LABEL + '.lock'
        lock = os.open(lockname, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600, dir_fd=parent)
        try:
            lockmeta = os.fstat(lock)
            if lockmeta.st_uid != os.getuid() or lockmeta.st_nlink != 1 or lockmeta.st_mode & 0o077:
                raise ValueError('Unsafe lifecycle lock')
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            if old is not None and dep.identity(target, 1024 * 1024) != (old_identity, old):
                raise ValueError('Installed agent changed before mutation')
            if old is None and os.path.lexists(target):
                raise ValueError('Agent appeared before installation')
            pending = list(target.parent.glob('.' + LABEL + '.intent-*.json'))
            if pending and action != 'reconcile':
                raise ValueError('Prior command intent requires operator reconciliation before further mutation')
            expected_job = (target, plistlib.loads(old)['ProgramArguments']) if old else None
            state = launch_state(run, domain, expected_job)
            if action == 'reconcile':
                if state != expected_state:
                    raise ValueError('Actual job state differs from requested reconciliation')
                if not pending:
                    raise ValueError('No unresolved command intent to reconcile')
                for intent in pending:
                    item = json.loads(read_private(intent))
                    current_digest = digest(old) if old is not None else dep.sha(expected_installed)
                    if old is None and (state != 'absent' or item.get('action') != 'install'):
                        raise ValueError('Missing agent can only reconcile an absent interrupted installation')
                    if item.get('domain') != domain or current_digest not in {item.get('installed_sha256'), item.get('proposed_sha256')}:
                        raise ValueError('Intent does not bind current agent')
                report['phase'] = 'operator-reconciled-' + state
            elif action in {'install', 'update', 'rollback'}:
                if state != 'absent':
                    raise ValueError('Stop exact app job and reconcile clean socket before replacement')
                dep.revalidate_inputs(review, expected_review)
                if old is not None:
                    # Exclusive backup means retries cannot silently overwrite audit history.
                    write_new(parent, f'.{LABEL}.{digest(old)}.before-{digest(payload)}-{uuid.uuid4().hex}.plist', old)
                write_new(parent, '.' + LABEL + '.intent-' + uuid.uuid4().hex + '.json',
                          json.dumps({'action': action, 'installed_sha256': digest(old) if old else None,
                                      'proposed_sha256': digest(payload), 'domain': domain,
                                      'outcome': 'requires-operator-reconciliation'}).encode())
                temp = f'.{LABEL}.{digest(payload)}.{uuid.uuid4().hex}.pending.plist'
                write_new(parent, temp, payload)
                current_parent = target.parent.lstat()
                if (current_parent.st_dev, current_parent.st_ino) != (metadata.st_dev, metadata.st_ino):
                    raise ValueError('LaunchAgents parent changed')
                if old is not None and dep.identity(target, 1024 * 1024) != (old_identity, old):
                    raise ValueError('Installed agent changed immediately before replacement')
                if old is None:
                    os.link(temp, target.name, src_dir_fd=parent, dst_dir_fd=parent, follow_symlinks=False)
                    os.unlink(temp, dir_fd=parent)
                else:
                    os.rename(temp, target.name, src_dir_fd=parent, dst_dir_fd=parent)
                os.fsync(parent)
                report['phase'] = 'installed-unloaded'
            elif action == 'start':
                if state != 'absent':
                    raise ValueError('Job already loaded; inspect status rather than retry')
                dep.revalidate_inputs(review, expected_review)
                write_new(parent, '.' + LABEL + '.intent-' + uuid.uuid4().hex + '.json',
                          json.dumps({'action': action, 'installed_sha256': digest(old), 'domain': domain,
                                      'outcome': 'requires-operator-reconciliation'}).encode())
                if dep.identity(target, 1024 * 1024) != (old_identity, old):
                    raise ValueError('Installed agent changed immediately before activation')
                commands = [['bootstrap', f'gui/{os.getuid()}', str(target)]]
                if not plistlib.loads(old)['RunAtLoad']:
                    commands.append(['kickstart', domain])
                for command in commands:
                    result = run(['/bin/launchctl', *command], capture_output=True, text=True, timeout=15)
                    if result.returncode:
                        raise ValueError('Activation outcome requires operator reconciliation; do not retry automatically')
                if launch_state(run, domain, expected_job) != 'loaded':
                    raise ValueError('Activation state remains unknown; reconcile intent')
                report['phase'] = 'loaded-not-qualified'
            else:
                if state != 'loaded':
                    raise ValueError('Job is absent; no stop action performed')
                write_new(parent, '.' + LABEL + '.intent-' + uuid.uuid4().hex + '.json',
                          json.dumps({'action': action, 'installed_sha256': digest(old), 'domain': domain,
                                      'outcome': 'requires-operator-reconciliation'}).encode())
                if dep.identity(target, 1024 * 1024) != (old_identity, old):
                    raise ValueError('Installed agent changed immediately before stop')
                result = run(['/bin/launchctl', 'bootout', domain], capture_output=True, text=True, timeout=15)
                if result.returncode:
                    raise ValueError('Stop outcome unknown; inspect exact job before retrying')
                if launch_state(run, domain, expected_job) != 'absent':
                    raise ValueError('Stop state remains unknown; reconcile intent')
                report['phase'] = 'unloaded-not-qualified'
            for intent in target.parent.glob('.' + LABEL + '.intent-*.json'):
                os.rename(intent.name, intent.name.replace('.intent-', '.reconciled-' if action == 'reconcile' else '.completed-'), src_dir_fd=parent, dst_dir_fd=parent)
            os.fsync(parent)
            report['installed_sha256'] = digest(read_private(target)) if target.exists() else None
            report['applied'] = True
        finally:
            os.close(lock)
    finally:
        os.close(parent)
    return report

def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument('--action', required=True, choices=['install', 'start', 'stop', 'status', 'update', 'rollback', 'reconcile'])
    parser.add_argument('--review', type=Path)
    parser.add_argument('--expected-review-sha256')
    parser.add_argument('--expected-installed-sha256')
    parser.add_argument('--expected-state', choices=['loaded', 'absent'])
    parser.add_argument('--start-at-login', action='store_true', help='Install/update/rollback only: opt into RunAtLoad; login/reboot remains unqualified')
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    try:
        print(json.dumps(execute(args.action, Path.home(), args.review, args.expected_review_sha256,
                                 args.expected_installed_sha256, args.apply, expected_state=args.expected_state, start_at_login=args.start_at_login), sort_keys=True))
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        parser.exit(1, f'{error}\n')
if __name__ == '__main__':
    main()
