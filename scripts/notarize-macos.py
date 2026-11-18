#!/usr/bin/env python3
"""Checkpoint a signed DMG and resume its original Apple notarization submission."""
import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import uuid


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def command(arguments, timeout=120):
    environment = dict(os.environ)
    for name in ('APPLE_API_PRIVATE_KEY', 'APPLE_API_KEY', 'APPLE_API_ISSUER'):
        environment.pop(name, None)
    try:
        return subprocess.run(arguments, env=environment, capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired):
        raise SystemExit('Notarization tool unavailable or timed out; credential output suppressed') from None


def verify_signature(dmg):
    if command(['codesign', '--verify', '--strict', str(dmg)]).returncode:
        raise SystemExit('The original signed notarization input failed signature verification')


def expected_receipt(plan_path, plan, dmg):
    return {'schema': 1, 'planSha256': digest(plan_path), 'sourceSha': plan['sourceSha'],
            'runId': str(plan['runId']), 'tag': plan['tag'], 'bundleVersion': plan['bundleVersion'],
            'fileName': dmg.name, 'dmgSha256': digest(dmg)}


def load_receipt(path, expected):
    try:
        receipt = json.loads(path.read_text())
        submission = str(uuid.UUID(receipt['submissionId']))
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        raise SystemExit('Missing or invalid original notarization receipt; do not resubmit') from None
    if receipt != {**expected, 'submissionId': submission}:
        raise SystemExit('Notarization input differs from the original signed image or release plan')
    return receipt


@contextmanager
def authentication():
    names = ('APPLE_API_PRIVATE_KEY', 'APPLE_API_KEY', 'APPLE_API_ISSUER')
    if any(not os.environ.get(name) for name in names):
        raise SystemExit('Notarization requires the environment API credentials')
    with tempfile.TemporaryDirectory(prefix='musicx-notary-', dir=os.environ.get('RUNNER_TEMP')) as directory:
        key = Path(directory) / 'key.p8'
        key.write_text(os.environ['APPLE_API_PRIVATE_KEY'].rstrip() + '\n')
        key.chmod(0o600)
        yield ['--key', str(key), '--key-id', os.environ['APPLE_API_KEY'],
               '--issuer', os.environ['APPLE_API_ISSUER'], '--output-format', 'json']


def submit(dmg, path, expected):
    if path.exists():
        receipt = load_receipt(path, expected)
        verify_signature(dmg)
        print('Using original notarization submission ' + receipt['submissionId'])
        return
    verify_signature(dmg)
    with authentication() as auth:
        result = command(['xcrun', 'notarytool', 'submit', str(dmg), *auth], timeout=300)
    if result.returncode:
        raise SystemExit('Notarization submission failed; inspect Apple history before retrying an ambiguous upload')
    try:
        submission = str(uuid.UUID(json.loads(result.stdout)['id']))
    except (ValueError, KeyError, TypeError, AttributeError):
        raise SystemExit('No valid submission ID returned; inspect Apple history before retrying') from None
    if digest(dmg) != expected['dmgSha256']:
        raise SystemExit('The signed image changed during notarization upload')
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps({**expected, 'submissionId': submission}, indent=2) + '\n')
    print('Recorded notarization submission ' + submission)


def wait(dmg, path, expected):
    receipt = load_receipt(path, expected)
    verify_signature(dmg)
    with authentication() as auth:
        result = command(['xcrun', 'notarytool', 'wait', receipt['submissionId'],
                          '--timeout', '20m', *auth], timeout=1300)
    response = {}
    if not result.returncode:
        try:
            response = json.loads(result.stdout)
        except ValueError:
            raise SystemExit('Invalid notarization status response; the original checkpoint is retained') from None
        if not isinstance(response, dict) or response.get('id') != receipt['submissionId']:
            raise SystemExit('Notarization status belongs to another submission')
    reported = response.get('status')
    state = 'Pending' if result.returncode == 124 else (
        reported if reported in ('Accepted', 'Invalid', 'In Progress') else 'Error')
    status = {'schema': 1, 'submissionId': receipt['submissionId'], 'tag': receipt['tag'],
              'sourceSha': receipt['sourceSha'], 'inputSha256': receipt['dmgSha256'],
              'state': state, 'toolExitCode': result.returncode}
    status_path = path.with_name('status.json')
    status_path.write_text(json.dumps(status, indent=2) + '\n')
    if result.returncode or state != 'Accepted':
        print('Notarization state: ' + state + '; original submission ' + receipt['submissionId'])
        raise SystemExit(124 if result.returncode == 124 else 1)
    for operation in ('staple', 'validate'):
        if command(['xcrun', 'stapler', operation, str(dmg)], timeout=180).returncode:
            status['state'] = 'Stapling failed'
            status_path.write_text(json.dumps(status, indent=2) + '\n')
            raise SystemExit('Notarization accepted but stapling failed; restore the original checkpoint and retry')
    status.update(state='Accepted and stapled', stapledSha256=digest(dmg))
    status_path.write_text(json.dumps(status, indent=2) + '\n')
    print('Apple notarization accepted and stapled: ' + receipt['submissionId'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('submit', 'verify', 'wait'))
    parser.add_argument('dmg', type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    plan_path = Path(os.environ['RELEASE_PLAN'])
    plan = json.loads(plan_path.read_text())
    if (plan['schema'] != 1 or plan['channel'] != 'stable'
            or plan['sourceSha'] != os.environ['SOURCE_SHA']
            or str(plan['runId']) != os.environ['GITHUB_RUN_ID']
            or plan['branch'] != os.environ['RELEASE_BRANCH']):
        raise SystemExit('Notarization release provenance mismatch')
    if not args.dmg.is_file():
        raise SystemExit('Original signed notarization input is missing; do not rebuild')
    path = Path(os.environ.get('NOTARY_RECEIPT', 'target/notary-input/submission.json'))
    expected = expected_receipt(plan_path, plan, args.dmg)
    if args.operation == 'submit':
        submit(args.dmg, path, expected)
    elif args.operation == 'verify':
        receipt = load_receipt(path, expected)
        verify_signature(args.dmg)
        print('Verified original notarization input for ' + receipt['submissionId'])
    else:
        wait(args.dmg, path, expected)


if __name__ == '__main__':
    main()
