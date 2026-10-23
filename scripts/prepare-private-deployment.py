#!/usr/bin/env python3
"""Generate disabled, uninstalled private deployment review plans; never execute inputs."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import re
import stat
import sys


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


bundle = load('private_deployment_bundle', 'package-macos-release.py')
launcher = load('private_deployment_launcher', 'make-launch-agent.py')
FORMAT = 'kAccounting-Extra-01.private-deployment-review.v1'
ROOT = Path('/Library/Application Support/kAccounting-Extra-01/private-transport')
CONTROL = Path('/private/var/run/kaccounting-private-transport')
LABEL = 'local.kAccounting-Extra-01.private-transport'


def sha(value):
    if not isinstance(value, str) or not re.fullmatch('[0-9a-f]{64}', value):
        raise ValueError('Explicit lowercase SHA-256 digest required')
    return value


def identity(path, maximum, executable=False):
    path = bundle.canonical(path)
    before = bundle.regular(path, maximum, executable)
    if before.st_mode & (stat.S_ISUID | stat.S_ISGID):
        raise ValueError('Privilege-bearing inputs refused')
    contents = bundle.read_checked(path, maximum)
    after = path.lstat()
    if bundle.identity(before) != bundle.identity(after):
        raise ValueError('Input identity changed')
    return {'path': str(path), 'sha256': hashlib.sha256(contents).hexdigest(),
            'bytes': len(contents), 'device': after.st_dev, 'inode': after.st_ino,
            'mode': oct(stat.S_IMODE(after.st_mode))}, contents


def staged_binary(path, expected, target):
    bundle.safe_directory(Path(path).parent, True)
    value, _ = identity(path, bundle.MAX_BINARY, True)
    if value['sha256'] != sha(expected):
        raise ValueError('Staged binary digest mismatch')
    bundle.macho(path, bundle.MAX_BINARY, target)
    return value


def runtime_snapshot(runtime, expected_manifest):
    """Capture verified bundle names and identities without executing any member."""
    bundle.verify(runtime, expected_manifest)
    result = {}
    for directory, dirs, names in os.walk(runtime, followlinks=False):
        for path in [Path(directory), *(Path(directory) / name for name in dirs + names)]:
            result[str(path.relative_to(runtime))] = list(bundle.identity(path.lstat()))
    return result


def prepare(runtime, expected_manifest, tailscaled, tailscaled_hash, tailscale,
            tailscale_hash, provenance, books_root, private_config, output, example_only=False):
    if not isinstance(example_only, bool):
        raise ValueError('Example-only must be an explicit boolean')
    expected_manifest = sha(expected_manifest)
    runtime = bundle.canonical(runtime)
    launcher.outside_source(runtime)
    verified = bundle.verify(runtime, expected_manifest)
    daemon = staged_binary(tailscaled, tailscaled_hash, verified['target'])
    client = staged_binary(tailscale, tailscale_hash, verified['target'])
    if (daemon['device'], daemon['inode']) == (client['device'], client['inode']):
        raise ValueError('Daemon and CLI must be distinct staged files')
    provenance_identity, raw = identity(provenance, 16384)
    bundle.safe_directory(Path(provenance).parent, True)
    if provenance_identity['mode'] != '0o600':
        raise ValueError('Provenance metadata must be private mode 600')
    metadata = json.loads(raw, object_pairs_hook=bundle.unique)
    required = {'version', 'source_url', 'source_revision', 'build_note', 'tailscaled_sha256', 'tailscale_sha256'}
    if (not isinstance(metadata, dict) or set(metadata) != required
            or not all(isinstance(value, str) for value in metadata.values())
            or not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+', metadata['version'])
            or metadata['source_url'] != 'https://github.com/tailscale/tailscale'
            or not re.fullmatch('[0-9a-f]{40}', metadata['source_revision'])
            or not 1 <= len(metadata['build_note']) <= 2000
            or any(ord(char) < 32 for char in metadata['build_note'])
            or metadata['tailscaled_sha256'] != daemon['sha256']
            or metadata['tailscale_sha256'] != client['sha256']):
        raise ValueError('Provenance must identify exact source, declared version and both hashes')
    books_root = bundle.canonical(books_root)
    private_config = bundle.canonical(private_config)
    helper_relative = verified.get('native_helper')
    if not helper_relative:
        raise ValueError('Development runtime must include the native helper')
    helper = runtime / helper_relative
    app = launcher.configuration(runtime / 'bin/kaccounting-backend', books_root,
                                 runtime / 'frontend', private_config, helper)
    books_identity = bundle.identity(bundle.safe_directory(books_root, True))
    config_identity = launcher.private_configuration_identity(private_config)
    runtime_identity = runtime_snapshot(runtime, expected_manifest)

    def validate_inputs():
        if runtime_snapshot(runtime, expected_manifest) != runtime_identity:
            raise ValueError('Runtime identities changed before publication')
        if (launcher.configuration(runtime / 'bin/kaccounting-backend', books_root,
                                   runtime / 'frontend', private_config, helper) != app
                or launcher.private_configuration_identity(private_config) != config_identity
                or bundle.identity(bundle.safe_directory(books_root, True)) != books_identity):
            raise ValueError('Launcher configuration or book collection changed before publication')
        if (staged_binary(tailscaled, tailscaled_hash, verified['target']) != daemon
                or staged_binary(tailscale, tailscale_hash, verified['target']) != client
                or identity(provenance, 16384)[0] != provenance_identity):
            raise ValueError('Transport or provenance changed before publication')
    daemon_plan = {
        'Label': LABEL, 'Disabled': True, 'UserName': 'root',
        'ProgramArguments': [str(ROOT / 'bin/tailscaled'), '--tun=userspace-networking',
                             '--statedir=' + str(ROOT / 'state'),
                             '--state=' + str(ROOT / 'state/tailscaled.state'),
                             '--socket=' + str(CONTROL / 'tailscaled.sock'), '--port=0'],
        'RunAtLoad': False, 'KeepAlive': False, 'Umask': 0o077,
        'WorkingDirectory': str(ROOT),
    }
    output = Path(output)
    if (not output.is_absolute() or '..' in output.parts
            or any(part.casefold() in {'launchagents', 'launchdaemons'} for part in output.parts)):
        raise ValueError('Review output must be a NEW absolute path outside launch directories')
    if any(output == protected or protected in output.parents for protected in (runtime, books_root)):
        raise ValueError('Review output must remain outside runtime bundles and book collections')
    launcher.outside_source(output)
    parent = bundle.canonical(output.parent)
    bundle.safe_directory(parent, True)
    parent_fd = launcher.directory_fd(parent)
    try:
        parent_identity = os.fstat(parent_fd)
        os.mkdir(output.name, mode=0o700, dir_fd=parent_fd)
        output_fd = os.open(output.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent_fd)
        try:
            root_identity = os.fstat(output_fd)
            def emit(name, data):
                bundle.safe_directory(parent, True)
                bundle.safe_directory(output, True)
                if ((parent.lstat().st_dev, parent.lstat().st_ino) != (parent_identity.st_dev, parent_identity.st_ino)
                        or (output.lstat().st_dev, output.lstat().st_ino) != (root_identity.st_dev, root_identity.st_ino)):
                    raise ValueError('Review directory identity changed')
                fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=output_fd)
                with os.fdopen(fd, 'wb') as file:
                    file.write(data); file.flush(); os.fsync(file.fileno())
            # Revalidate inputs immediately before publishing plans. Never adopt a changed runtime.
            validate_inputs()
            daemon_bytes, app_bytes = plistlib.dumps(daemon_plan), plistlib.dumps(app)
            report = {
                'format': FORMAT, 'installed': False, 'started': False, 'qualified': False,
                'example_only': example_only, 'deployment_ready': False,
                'plan_sha256': {'disabled-root-launchdaemon.plist': hashlib.sha256(daemon_bytes).hexdigest(),
                                'disabled-user-launchagent.plist': hashlib.sha256(app_bytes).hexdigest()},
                'signed': False, 'serve_configured': False, 'funnel_enabled': False,
                'runtime': {'path': str(runtime), **verified}, 'transport': {'tailscaled': daemon, 'tailscale': client},
                'provenance': metadata, 'provenance_identity': provenance_identity,
                'provenance_is_operator_assertion_not_build_attestation': True,
                'private_configuration_identity': config_identity,
                'input_snapshot': {'version': 1, 'private_config': str(private_config),
                                   'runtime_identity': runtime_identity},
                'books_root_identity': {'path': str(books_root), 'device': books_identity[0], 'inode': books_identity[1]},
                'future_root_install': {'root': str(ROOT), 'control_parent': str(CONTROL),
                    'launchdaemon': '/Library/LaunchDaemons/' + LABEL + '.plist',
                    'required_owner': 'root:wheel', 'directory_mode': '0o700', 'binary_mode': '0o700',
                    'state_and_control_must_be_new': True, 'adopt_existing_gui_state': False},
                'requirements': [
                    'Operator must review all paths, source provenance, hashes and development signing limitations.',
                    'No installer is emitted. A separately reviewed installer must revalidate all identities under administrator authorization.',
                    'Never replace existing GUI binaries, daemon state, sockets, login items, books or Keychain items.',
                    'Future root directories, state and control socket must be new, root-owned, mode 700; refuse existing names.',
                    'Future installed binaries must be root-owned single-link regular files, mode 700 and match these hashes.',
                    'New-node login and private Serve require separate operator review; no Funnel, public exposure or TCP fallback.',
                    'Before production qualify real root-peer Unix ingress, private identity refusals, Safari, Keychain, logout/login and reboot.',
                    'Rollback: stop only the exact newly reviewed jobs and Serve route; preserve books, keys and new daemon state for review.',
                    'Do not delete persistent state or touch the original GUI; document explicit operator-approved cleanup separately.',
                ],
            }
            emit('disabled-root-launchdaemon.plist', daemon_bytes)
            emit('disabled-user-launchagent.plist', app_bytes)
            review_bytes = bundle.encoded(report)
            emit('review.json', review_bytes)
            completion_bytes = bundle.encoded({'format': FORMAT, 'review_sha256': hashlib.sha256(review_bytes).hexdigest(), 'installed': False, 'started': False, 'qualified': False})
            # Completion is the publication boundary. Repeat every input check
            # after writing plans, immediately before exclusive marker creation.
            validate_inputs()
            emit('complete.json', completion_bytes)
            os.fsync(output_fd)
        finally:
            os.close(output_fd)
        os.fsync(parent_fd)
    finally:
        os.close(parent_fd)
    return {'output': str(output), 'review_sha256': hashlib.sha256(review_bytes).hexdigest(), 'example_only': example_only, 'installed': False, 'started': False, 'qualified': False}


def verify_review(path, expected_review):
    """Offline artifact integrity check only; never inspect sources or assert installation."""
    expected_review = sha(expected_review)
    root = bundle.canonical(path)
    bundle.safe_directory(root, True)
    expected_names = {'disabled-root-launchdaemon.plist', 'disabled-user-launchagent.plist', 'review.json', 'complete.json'}
    if set(os.listdir(root)) != expected_names:
        raise ValueError('Review contains unknown or missing files')
    files = {}
    for name in expected_names:
        metadata, contents = identity(root / name, 1024 * 1024)
        if metadata['mode'] != '0o600':
            raise ValueError('Review files require exact private modes')
        files[name] = contents
    report = json.loads(files['review.json'], object_pairs_hook=bundle.unique)
    marker = json.loads(files['complete.json'], object_pairs_hook=bundle.unique)
    if (not isinstance(report, dict) or not isinstance(marker, dict)
            or marker != {'format': FORMAT, 'review_sha256': expected_review, 'installed': False, 'started': False, 'qualified': False}
            or hashlib.sha256(files['review.json']).hexdigest() != expected_review
            or report.get('format') != FORMAT or not isinstance(report.get('example_only'), bool)
            or any(report.get(key) is not False for key in ['installed', 'started', 'qualified', 'deployment_ready', 'signed', 'serve_configured', 'funnel_enabled'])):
        raise ValueError('Trusted review digest or disabled status mismatch')
    hashes = report.get('plan_sha256')
    plan_names = expected_names - {'review.json', 'complete.json'}
    if not isinstance(hashes, dict) or set(hashes) != plan_names:
        raise ValueError('Plan hash binding missing')
    for name in plan_names:
        if hashlib.sha256(files[name]).hexdigest() != sha(hashes[name]):
            raise ValueError('Reviewed launch plan changed')
        plan = plistlib.loads(files[name])
        if not isinstance(plan, dict) or plan.get('Disabled') is not True:
            raise ValueError('Launch plan must remain disabled')
    return {'review_sha256': expected_review, 'artifact_integrity_verified': True,
            'example_only': report['example_only'], 'source_inputs_revalidated': False,
            'installed': False, 'started': False, 'qualified': False}


def revalidate_inputs(path, expected_review):
    """Read-only stale-input check; never installation or readiness qualification."""
    result = verify_review(path, expected_review)
    root = bundle.canonical(path)
    _, raw = identity(root / 'review.json', 1024 * 1024)
    if hashlib.sha256(raw).hexdigest() != sha(expected_review):
        raise ValueError('Review changed before input validation')
    report = json.loads(raw, object_pairs_hook=bundle.unique)
    snapshot = report.get('input_snapshot')
    if (report['example_only'] or not isinstance(snapshot, dict)
            or set(snapshot) != {'version', 'private_config', 'runtime_identity'}
            or type(snapshot['version']) is not int or snapshot['version'] != 1):
        raise ValueError('Requires non-example review with pinned input snapshot')
    runtime = bundle.canonical(report['runtime']['path'])
    launcher.outside_source(runtime)
    manifest = sha(report['runtime']['manifest_sha256'])
    expected_runtime = snapshot['runtime_identity']
    if not isinstance(expected_runtime, dict) or not expected_runtime:
        raise ValueError('Missing runtime identities')
    books = report['books_root_identity']
    books_root = bundle.canonical(books['path'])
    config = bundle.canonical(snapshot['private_config'])
    helper = runtime / report['runtime']['native_helper']
    # Repeat the full pass to catch observed substitutions across validation.
    for _ in range(2):
        if runtime_snapshot(runtime, manifest) != expected_runtime:
            raise ValueError('Runtime identities changed since review')
        current = bundle.verify(runtime, manifest)
        if {'path': str(runtime), **current} != report['runtime']:
            raise ValueError('Runtime verification contract changed')
        for name in ['tailscaled', 'tailscale']:
            pinned = report['transport'][name]
            if staged_binary(pinned['path'], pinned['sha256'], current['target']) != pinned:
                raise ValueError('Transport identity changed since review')
        pinned = report['provenance_identity']
        bundle.safe_directory(Path(pinned['path']).parent, True)
        provenance, contents = identity(pinned['path'], 16384)
        if provenance != pinned or json.loads(contents, object_pairs_hook=bundle.unique) != report['provenance']:
            raise ValueError('Provenance changed since review')
        if launcher.private_configuration_identity(config) != report['private_configuration_identity']:
            raise ValueError('Private configuration changed since review')
        metadata = bundle.safe_directory(books_root, True)
        if (metadata.st_dev, metadata.st_ino) != (books['device'], books['inode']):
            raise ValueError('Books collection identity changed since review')
        app = launcher.configuration(runtime / 'bin/kaccounting-backend', books_root,
                                     runtime / 'frontend', config, helper)
        _, raw_plan = identity(root / 'disabled-user-launchagent.plist', 1024 * 1024)
        if plistlib.loads(raw_plan) != app:
            raise ValueError('Launcher configuration changed since review')
    verify_review(root, expected_review)
    return {**result, 'source_inputs_revalidated': True, 'deployment_ready': False,
            'atomic_installation_boundary': False}


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    for name in ['runtime', 'expected-manifest', 'tailscaled', 'tailscaled-hash', 'tailscale', 'tailscale-hash', 'provenance', 'books-root', 'private-config', 'output']:
        parser.add_argument('--' + name)
    parser.add_argument('--example-only', action='store_true', help='Mark all outputs as nondeployable illustrative review inputs')
    parser.add_argument('--verify', help='Only verify an existing review package against its independently retained digest')
    parser.add_argument('--expected-review-sha256')
    parser.add_argument('--revalidate-inputs', action='store_true', help='With --verify, recheck pinned non-example inputs without writes or execution')
    args = parser.parse_args()
    supplied = vars(args)
    verify, expected = supplied.pop('verify'), supplied.pop('expected_review_sha256')
    revalidate = supplied.pop('revalidate_inputs')
    if verify:
        if not expected or any(value for key, value in supplied.items()):
            parser.error('Verification requires only --verify and --expected-review-sha256')
        result = revalidate_inputs(verify, expected) if revalidate else verify_review(verify, expected)
    else:
        if expected or revalidate or any(value is None for value in supplied.values()):
            parser.error('Preparation requires every input argument; verification options cannot be mixed')
        result = prepare(**supplied)
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, TypeError, KeyError, plistlib.InvalidFileException):
        print('Private deployment plan refused; inputs preserved; nothing installed or started. Partial review output, if any, is retained.', file=sys.stderr)
        sys.exit(1)
