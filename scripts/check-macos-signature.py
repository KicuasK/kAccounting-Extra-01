#!/usr/bin/env python3
"""Read-only Developer ID preflight for one executable; never sign/install or read keys."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys

spec = importlib.util.spec_from_file_location('runtime_bundle', Path(__file__).with_name('package-macos-release.py'))
bundle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bundle)


def metadata(text):
    if len(text) > 65536:
        raise ValueError('Signature metadata capacity exceeded.')
    fields = {}
    for line in text.splitlines():
        for name in ('Identifier', 'TeamIdentifier', 'Signature', 'CodeDirectory'):
            if line.startswith(name + '=') or (name == 'CodeDirectory' and line.startswith(name + ' ')):
                if name in fields:
                    raise ValueError('Duplicate signature identity field.')
                fields[name] = line[len(name) + 1:]
    flags = re.search(r'\bflags=0x([0-9a-fA-F]+)\b', fields.get('CodeDirectory', ''))
    return {
        'identifier': fields.get('Identifier'),
        'team_id': fields.get('TeamIdentifier'),
        'ad_hoc': fields.get('Signature') == 'adhoc',
        'developer_id_authority': any(line.startswith('Authority=Developer ID Application: ') for line in text.splitlines()),
        'hardened_runtime': bool(flags and int(flags[1], 16) & 0x10000),
    }


def inspect(binary, identifier, team_id=None, runner=None):
    if not re.fullmatch(r'[A-Za-z][A-Za-z0-9-]*(?:\.[A-Za-z0-9-]+)+', identifier):
        raise ValueError('Expected bundle identifier is required.')
    if team_id is not None and not re.fullmatch('[A-Z0-9]{10}', team_id):
        raise ValueError('Expected ten-character team ID is required.')
    path = bundle.canonical(binary)
    bundle.regular(path, bundle.MAX_BINARY, True)
    before = hashlib.sha256(bundle.read_checked(path, bundle.MAX_BINARY)).hexdigest()
    run = runner or subprocess.run
    arguments = dict(capture_output=True, text=True, timeout=15, check=False,
                     env={'PATH': '/usr/bin:/bin', 'LC_ALL': 'C'})
    verified = run(['/usr/bin/codesign', '--verify', '--strict', '--verbose=2', '--', str(path)], **arguments)
    displayed = run(['/usr/bin/codesign', '--display', '--verbose=4', '--', str(path)], **arguments)
    details = metadata(displayed.stderr) if displayed.returncode == 0 else metadata('')
    # Metadata labels alone cannot establish Apple certificate provenance.
    requirement_verified = False
    if team_id is not None:
        requirement = ('anchor apple generic and '
                       'certificate leaf[field.1.2.840.113635.100.6.1.13] exists and '
                       'certificate leaf[subject.OU] = "' + team_id + '" and '
                       'identifier "' + identifier + '"')
        tested = run(['/usr/bin/codesign', '--verify', '--strict', '-R', requirement, '--', str(path)], **arguments)
        requirement_verified = tested.returncode == 0
    after = hashlib.sha256(bundle.read_checked(path, bundle.MAX_BINARY)).hexdigest()
    if before != after:
        raise ValueError('Executable changed during signature preflight.')
    reasons = []
    if verified.returncode != 0 or displayed.returncode != 0:
        reasons.append('signature_verification_failed')
    if details['ad_hoc'] or not details['developer_id_authority']:
        reasons.append('developer_id_application_required')
    if details['identifier'] != identifier:
        reasons.append('identifier_mismatch')
    if team_id is None:
        reasons.append('expected_team_id_required')
    elif details['team_id'] != team_id:
        reasons.append('team_id_mismatch')
    if team_id is not None and not requirement_verified:
        reasons.append('developer_id_requirement_failed')
    if not details['hardened_runtime']:
        reasons.append('hardened_runtime_missing')
    return {
        'format': 'kAccounting-Extra-01.signature-preflight.v1',
        'sha256': before, 'signature_verified': verified.returncode == 0 and displayed.returncode == 0,
        'developer_id_qualified': not reasons, 'developer_id_requirement_verified': requirement_verified,
        'reasons': reasons, **details,
        'notarization_checked': False, 'production_keychain_qualified': False,
        'login_reboot_qualified': False, 'private_proxy_qualified': False,
        'installed': False, 'started': False,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--identifier', required=True)
    parser.add_argument('--team-id')
    args = parser.parse_args()
    result = inspect(args.binary, args.identifier, args.team_id)
    print(json.dumps(result, sort_keys=True))
    return 0 if result['developer_id_qualified'] else 1


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.SubprocessError):
        print('Signature preflight refused: invalid input, changed executable, unavailable codesign or timeout.', file=sys.stderr)
        sys.exit(2)
