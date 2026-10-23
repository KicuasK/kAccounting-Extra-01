#!/usr/bin/env python3
"""Prepare a disabled transport-only bootstrap review; never execute or log in."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import sys

spec = importlib.util.spec_from_file_location('transport_bootstrap_base', Path(__file__).with_name('prepare-private-deployment.py'))
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
bundle = base.bundle
identity = base.identity
FORMAT = 'kAccounting-Extra-01.private-transport-bootstrap-review.v1'
STATUS = {key: False for key in ('installed', 'started', 'qualified', 'deployment_ready', 'signed', 'serve_configured', 'funnel_enabled', 'login_performed', 'example_only')}


def snapshot(tailscaled, tailscaled_hash, tailscale, tailscale_hash, provenance, target):
    if target not in bundle.TARGETS:
        raise ValueError('Known target required')
    transport = {name: base.staged_binary(path, digest, target) for name, path, digest in
                 (('tailscaled', tailscaled, tailscaled_hash), ('tailscale', tailscale, tailscale_hash))}
    if any(item['mode'] != '0o700' for item in transport.values()):
        raise ValueError('Transport requires exact private executable mode')
    if len({(item['device'], item['inode']) for item in transport.values()}) != 2:
        raise ValueError('Distinct transport files required')
    identity, raw = base.identity(provenance, 16384)
    bundle.safe_directory(Path(identity['path']).parent, True)
    metadata = json.loads(raw, object_pairs_hook=bundle.unique)
    required = {'version', 'source_url', 'source_revision', 'build_note', 'tailscaled_sha256', 'tailscale_sha256'}
    if (identity['mode'] != '0o600' or not isinstance(metadata, dict) or set(metadata) != required
            or not all(isinstance(v, str) for v in metadata.values())
            or not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+', metadata['version'])
            or metadata['source_url'] != 'https://github.com/tailscale/tailscale'
            or not re.fullmatch('[0-9a-f]{40}', metadata['source_revision'])
            or not 1 <= len(metadata['build_note']) <= 2000
            or any(ord(c) < 32 for c in metadata['build_note'])
            or any(metadata[name + '_sha256'] != transport[name]['sha256'] for name in transport)):
        raise ValueError('Exact private transport provenance required')
    return {'format': FORMAT, **STATUS, 'target': target, 'transport': transport,
            'provenance': metadata, 'provenance_identity': identity,
            'provenance_is_operator_assertion_not_build_attestation': True,
            'destination': str(base.ROOT), 'control_parent': str(base.CONTROL),
            'state_and_control_must_be_new': True, 'adopt_existing_gui_state': False}


def prepare(tailscaled, tailscaled_hash, tailscale, tailscale_hash, provenance, target, output):
    report = snapshot(tailscaled, tailscaled_hash, tailscale, tailscale_hash, provenance, target)
    output = Path(output)
    if not output.is_absolute() or '..' in output.parts or any(p.casefold() in {'launchagents', 'launchdaemons'} for p in output.parts):
        raise ValueError('New absolute non-launch output required')
    base.launcher.outside_source(output)
    parent = bundle.canonical(output.parent)
    bundle.safe_directory(parent, True)
    fd = base.launcher.directory_fd(parent)
    try:
        parent_id = os.fstat(fd)
        os.mkdir(output.name, 0o700, dir_fd=fd)
        child = os.open(output.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
        try:
            child_id = os.fstat(child)
            def check():
                if ((parent.stat().st_dev, parent.stat().st_ino) != (parent_id.st_dev, parent_id.st_ino)
                        or (output.lstat().st_dev, output.lstat().st_ino) != (child_id.st_dev, child_id.st_ino)):
                    raise ValueError('Review directory changed')
                bundle.safe_directory(parent, True)
                bundle.safe_directory(output, True)
                if snapshot(tailscaled, tailscaled_hash, tailscale, tailscale_hash, provenance, target) != report:
                    raise ValueError('Source changed before publication')
            def emit(name, data):
                check()
                dest = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=child)
                with os.fdopen(dest, 'wb') as stream:
                    stream.write(data); stream.flush(); os.fsync(stream.fileno())
            raw = (json.dumps(report, sort_keys=True) + '\n').encode()
            digest = hashlib.sha256(raw).hexdigest()
            emit('review.json', raw)
            emit('complete.json', (json.dumps({'format': FORMAT, 'review_sha256': digest, **STATUS}, sort_keys=True) + '\n').encode())
            check(); os.fsync(child)
        finally:
            os.close(child)
        os.fsync(fd)
    finally:
        os.close(fd)
    return {'review_sha256': digest, 'output': str(output), **STATUS}


def revalidate_inputs(path, expected_review):
    expected_review = base.sha(expected_review)
    root = bundle.canonical(path)
    bundle.safe_directory(root, True)
    if stat.S_IMODE(root.stat().st_mode) != 0o700 or set(os.listdir(root)) != {'review.json', 'complete.json'}:
        raise ValueError('Exact private bootstrap review required')
    values = {}
    for name in ('review.json', 'complete.json'):
        ident, raw = base.identity(root / name, 1024 * 1024)
        if ident['mode'] != '0o600':
            raise ValueError('Private review files required')
        if name == 'review.json' and hashlib.sha256(raw).hexdigest() != expected_review:
            raise ValueError('External review digest mismatch')
        values[name] = json.loads(raw, object_pairs_hook=bundle.unique)
    report = values['review.json']
    if values['complete.json'] != {'format': FORMAT, 'review_sha256': expected_review, **STATUS}:
        raise ValueError('Completion mismatch')
    transport = report['transport']
    current = snapshot(transport['tailscaled']['path'], transport['tailscaled']['sha256'], transport['tailscale']['path'], transport['tailscale']['sha256'], report['provenance_identity']['path'], report['target'])
    if report != current:
        raise ValueError('Bootstrap inputs or disabled contract changed')
    return {'review_sha256': expected_review, 'artifact_integrity_verified': True, 'source_inputs_revalidated': True, **STATUS}


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument('--verify')
    parser.add_argument('--expected-review-sha256')
    for name in ('tailscaled', 'tailscaled-hash', 'tailscale', 'tailscale-hash', 'provenance', 'target', 'output'):
        parser.add_argument('--' + name)
    args = vars(parser.parse_args())
    path, digest = args.pop('verify'), args.pop('expected_review_sha256')
    if path:
        if any(v is not None for v in args.values()):
            raise ValueError('Verification must not include preparation arguments')
        result = revalidate_inputs(path, digest)
    else:
        if digest is not None or any(v is None for v in args.values()):
            raise ValueError('All explicit preparation inputs required')
        result = prepare(**args)
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, TypeError, KeyError):
        print('Transport bootstrap review refused; no service executed.', file=sys.stderr)
        sys.exit(1)
