#!/usr/bin/env python3
"""Export validated plan metadata and bind immutable build artifacts to that plan."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re

FILES = ('MusicX-macos-arm64.dmg', 'MusicX-macos-arm64.dmg.sha256', 'appcast.xml',
         'appcast-all.xml', 'appcast-all.base')


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('env', 'record', 'verify'))
    parser.add_argument('--directory', type=Path, default=Path('target/distribution'))
    args = parser.parse_args()
    plan_path = Path(os.environ['RELEASE_PLAN'])
    plan = json.loads(plan_path.read_text())
    if (plan['schema'] != 1 or plan['sourceSha'] != os.environ['SOURCE_SHA']
            or str(plan['runId']) != os.environ['GITHUB_RUN_ID']
            or plan['branch'] != os.environ['RELEASE_BRANCH']):
        raise SystemExit('Release artifact provenance mismatch')
    if args.mode == 'env':
        numeric = r'(?:0|[1-9]\d*)'
        if (not re.fullmatch(rf'v{numeric}\.{numeric}\.{numeric}(?:-preview\.[1-9]\d*)?', plan['tag'])
                or plan['version'] != plan['tag'][1:]
                or plan['shortVersion'] != plan['version'].split('-')[0]
                or plan['bundleVersion'] != str(1000000 + int(os.environ['GITHUB_RUN_NUMBER']))
                or not re.fullmatch(r'update-feed(?:-\d+(?:\.\d+)?\.x)?', plan['feedTag'])):
            raise SystemExit('Invalid release plan metadata')
        values = {'RELEASE_TAG': plan['tag'], 'MUSICX_RELEASE_VERSION': plan['version'],
                  'MUSICX_SHORT_VERSION': plan['shortVersion'], 'MUSICX_BUNDLE_VERSION': plan['bundleVersion'],
                  'UPDATE_FEED_TAG': plan['feedTag'],
                  'MUSICX_UPDATE_FEED_URL': f'https://github.com/{os.environ["GITHUB_REPOSITORY"]}/releases/download/{plan["feedTag"]}/appcast.xml'}
        with Path(os.environ['GITHUB_ENV']).open('a') as output:
            for key, value in values.items():
                output.write(f'{key}={value}\n')
        return
    receipt_path = args.directory / 'validated-artifacts.json'
    receipt = {'schema': 1, 'planSha256': digest(plan_path), 'sourceSha': plan['sourceSha'], 'tag': plan['tag'],
               'signingMode': 'developer-id' if plan['channel'] == 'stable' else 'ad-hoc',
               'files': {name: digest(args.directory / name) for name in FILES}}
    if args.mode == 'record':
        receipt_path.write_text(json.dumps(receipt, indent=2) + '\n')
    elif json.loads(receipt_path.read_text()) != receipt:
        raise SystemExit('Release artifacts differ from the original signed build')


if __name__ == '__main__':
    main()
