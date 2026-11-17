#!/usr/bin/env python3
"""Resume upload, processing and distribution to one existing Internal TestFlight group."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode, urlsplit
from urllib.request import Request, urlopen

API = 'https://api.appstoreconnect.apple.com'


def api_path(url):
    parsed = urlsplit(url)
    if parsed.scheme != 'https' or parsed.netloc != 'api.appstoreconnect.apple.com':
        raise ValueError('Unexpected App Store Connect relationship URL')
    return parsed.path + ('?' + parsed.query if parsed.query else '')


def validate_receipt(receipt, plan, package):
    with package.open('rb') as stream:
        package_digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    if (receipt['source_sha'] != plan['sourceSha'] or receipt['version'] != plan['shortVersion']
            or receipt['build'] != plan['bundleVersion'] or receipt['prepare_only']
            or not receipt['sandbox'] or receipt['sparkle'] or receipt['bundle_id'] != 'com.popomore.musicx'
            or package_digest != receipt['sha256']):
        raise ValueError('TestFlight package does not match the original release plan')


class Apple:
    def __init__(self, key):
        self.key, self.token, self.expires = key, None, 0

    def authenticate(self):
        if self.token and self.expires > time.time() + 60:
            return
        result = subprocess.run(['xcrun', 'altool', '--generate-jwt', '--apiKey', os.environ['APPLE_API_KEY'],
                                 '--apiIssuer', os.environ['APPLE_API_ISSUER'], '--p8-file-path', str(self.key)],
                                capture_output=True, text=True)
        for token in re.findall(r'eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+', result.stdout + result.stderr):
            payload = token.split('.')[1]
            claims = json.loads(base64.urlsafe_b64decode(payload + '=' * (-len(payload) % 4)))
            if not result.returncode and claims.get('aud') == 'appstoreconnect-v1' and claims['exp'] > time.time():
                self.token, self.expires = token, claims['exp']
                # Do not print JWTs outside the Actions masking protocol.
                if os.environ.get('GITHUB_ACTIONS') == 'true':
                    print('::add-mask::' + token, flush=True)
                return
        raise RuntimeError('Could not authenticate with App Store Connect')

    def request(self, path, method='GET', payload=None):
        self.authenticate()
        if not path.startswith('/v1/'):
            raise ValueError('Unexpected Apple API path')
        request = Request(API + path, data=None if payload is None else json.dumps(payload).encode(),
                          headers={'Authorization': 'Bearer ' + self.token, 'Content-Type': 'application/json'}, method=method)
        for attempt in range(3):
            try:
                with urlopen(request, timeout=60) as response:
                    body = response.read()
                    return json.loads(body) if body else {}
            except HTTPError as error:
                if method == 'GET' and attempt < 2 and (error.code >= 500 or error.code == 429):
                    time.sleep(5 * (attempt + 1))
                    continue
                # Report status/code, never raw credentials or reflected request bodies.
                try:
                    codes = [entry.get('code', 'UNKNOWN') for entry in json.loads(error.read()).get('errors', [])]
                except (ValueError, KeyError):
                    codes = []
                raise RuntimeError(f'Apple API {method} {path.split("?")[0]} failed: HTTP {error.code}, codes {codes}') from None
            except (URLError, TimeoutError):
                if method == 'GET' and attempt < 2:
                    time.sleep(5 * (attempt + 1))
                    continue
                raise RuntimeError('Apple API connection failed; rerun this job to reconcile delivery') from None

    def collection(self, path):
        values = []
        while path:
            result = self.request(path)
            values.extend(result['data'])
            next_url = result.get('links', {}).get('next')
            path = api_path(next_url) if next_url else None
        return values

    def build(self, app, version, build_number):
        query = urlencode({'filter[app]': app, 'filter[version]': build_number, 'limit': 200})
        builds = self.collection('/v1/builds?' + query)
        matches = []
        for build in builds:
            pre_release = self.request(api_path(build['relationships']['preReleaseVersion']['links']['related']))['data']
            if pre_release['attributes']['version'] != version or pre_release['attributes']['platform'] != 'MAC_OS':
                raise ValueError('Build number already belongs to another version or platform')
            matches.append(build)
        if len(matches) > 1:
            raise ValueError('Multiple App Store Connect builds match this release')
        return matches[0] if matches else None


def distribute(apple, build, group_id):
    group = apple.request('/v1/betaGroups/' + group_id)['data']
    if not group['attributes']['isInternalGroup'] or group['attributes'].get('publicLinkEnabled'):
        raise ValueError('Only an existing Internal TestFlight group is allowed')
    group_app = apple.request(api_path(group['relationships']['app']['links']['related']))['data']['id']
    if group_app != os.environ['TESTFLIGHT_APP_ID']:
        raise ValueError('The Internal group belongs to another app')
    if build['attributes'].get('usesNonExemptEncryption') is None:
        # MusicX uses only exempt standard TLS/AES; this declaration was confirmed by the owner.
        apple.request('/v1/builds/' + build['id'], 'PATCH', {
            'data': {'type': 'builds', 'id': build['id'], 'attributes': {'usesNonExemptEncryption': False}}})
    elif build['attributes']['usesNonExemptEncryption']:
        raise ValueError('Unexpected export-compliance declaration; owner review required')
    existing = apple.collection('/v1/betaGroups/' + group_id + '/relationships/builds')
    if build['id'] not in {item['id'] for item in existing}:
        apple.request('/v1/builds/' + build['id'] + '/relationships/betaGroups', 'POST',
                      {'data': [{'type': 'betaGroups', 'id': group_id}]})
    return group


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('package', type=Path)
    args = parser.parse_args()
    plan = json.loads(Path(os.environ['RELEASE_PLAN']).read_text())
    if plan['branch'] != 'main' or plan['channel'] != 'preview':
        raise SystemExit('Automatic TestFlight distribution is restricted to main Preview builds')
    receipt = json.loads(args.package.with_suffix('.pkg.json').read_text())
    validate_receipt(receipt, plan, args.package)
    status_path = args.package.parent / 'testflight-status.json'
    status = {'tag': plan['tag'], 'source_sha': plan['sourceSha'], 'version': plan['shortVersion'],
              'build': plan['bundleVersion'], 'package_sha256': receipt['sha256'], 'state': 'starting'}
    def save(**values):
        status.update(values)
        status_path.write_text(json.dumps(status, indent=2) + '\n')
    save(state='starting')
    os.umask(0o077)
    with tempfile.TemporaryDirectory(prefix='musicx-apple-api-', dir=os.environ.get('RUNNER_TEMP')) as directory:
        key = Path(directory) / ('AuthKey_' + os.environ['APPLE_API_KEY'] + '.p8')
        key.write_text(os.environ['APPLE_API_PRIVATE_KEY'])
        apple = Apple(key)
        find = lambda: apple.build(os.environ['TESTFLIGHT_APP_ID'], plan['shortVersion'], plan['bundleVersion'])
        build = find()
        if build is None:
            auth = ['--apiKey', os.environ['APPLE_API_KEY'], '--apiIssuer', os.environ['APPLE_API_ISSUER'],
                    '--p8-file-path', str(key)]
            for operation in ('--validate-app', '--upload-app'):
                # Plain output avoids altool's JSON formatter crash after an accepted delivery.
                result = subprocess.run(['xcrun', 'altool', operation, '--type', 'macos', '-f', str(args.package), *auth],
                                        capture_output=True, text=True)
                if result.returncode and operation == '--validate-app':
                    save(state='validation_failed')
                    raise SystemExit('Apple package validation failed; upload was not attempted')
                if operation == '--upload-app':
                    # An ambiguous upload response is reconciled through the API below.
                    save(state='upload_sent', upload_exit_code=result.returncode)
                print(operation.removeprefix('--') + ' finished', flush=True)
        deadline = time.monotonic() + 1200
        assigned = False
        while time.monotonic() < deadline:
            build = find()
            if build:
                state = build['attributes']['processingState']
                save(state='processing', build_id=build['id'], processing_state=state)
                if state in ('INVALID', 'FAILED') or build['attributes']['expired']:
                    raise SystemExit('Apple rejected or expired the build; check App Store Connect')
                if state == 'VALID':
                    if not assigned:
                        distribute(apple, build, os.environ['TESTFLIGHT_GROUP_ID'])
                        assigned = True
                    detail = apple.request(api_path(build['relationships']['buildBetaDetail']['links']['related']))['data']
                    internal = detail['attributes']['internalBuildState']
                    members = apple.collection('/v1/betaGroups/' + os.environ['TESTFLIGHT_GROUP_ID'] + '/relationships/builds')
                    save(internal_state=internal, group_id=os.environ['TESTFLIGHT_GROUP_ID'])
                    if internal == 'IN_BETA_TESTING' and build['id'] in {item['id'] for item in members}:
                        save(state='internal_available')
                        print(f'TestFlight {plan["shortVersion"]} ({plan["bundleVersion"]}) is available in the existing Internal group')
                        return
            time.sleep(30)
        save(state='processing_timeout')
        raise SystemExit('Upload has not reached Internal availability; retry this job to reconcile the existing build')


if __name__ == '__main__':
    main()
