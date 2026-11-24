#!/usr/bin/env python3
"""Import environment-scoped signing material into a disposable CI keychain."""
import argparse
import base64
import os
from pathlib import Path
import re
import secrets
import subprocess


def run(*args):
    result = subprocess.run(args, capture_output=True, text=True)
    if result.returncode:
        # Never render a command line containing an import or keychain password.
        raise SystemExit(f'{args[0]} signing setup failed (exit {result.returncode})')
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('developer-id', 'testflight'))
    args = parser.parse_args()
    os.umask(0o077)
    stage = Path(os.environ['RUNNER_TEMP']) / 'musicx-signing'
    stage.mkdir(exist_ok=True)
    keychain = stage / 'release.keychain-db'
    password = secrets.token_hex(24)
    print(f'::add-mask::{password}')
    run('security', 'create-keychain', '-p', password, str(keychain))
    run('security', 'set-keychain-settings', '-lut', '21600', str(keychain))
    run('security', 'unlock-keychain', '-p', password, str(keychain))
    names = [('APPLE_CERTIFICATE', 'APPLE_CERTIFICATE_PASSWORD', 'Developer ID Application: ', 'MUSICX_CODESIGN_IDENTITY')]
    if args.mode == 'testflight':
        names = [('STORE_APP_CERTIFICATE', 'STORE_APP_CERTIFICATE_PASSWORD', '3rd Party Mac Developer Application: ', 'STORE_APP_IDENTITY'),
                 ('STORE_INSTALLER_CERTIFICATE', 'STORE_INSTALLER_CERTIFICATE_PASSWORD', '3rd Party Mac Developer Installer: ', 'STORE_INSTALLER_IDENTITY')]
    exported = {'MUSICX_SIGNING_KEYCHAIN': str(keychain)}
    for name, password_name, prefix, output_name in names:
        certificate = stage / (name + '.p12')
        certificate.write_bytes(base64.b64decode(os.environ[name], validate=True))
        try:
            run('security', 'import', str(certificate), '-f', 'pkcs12', '-k', str(keychain),
                '-P', os.environ[password_name], '-T', '/usr/bin/codesign', '-T', '/usr/bin/productbuild')
        finally:
            certificate.unlink(missing_ok=True)
        identities = run('security', 'find-identity', '-v', str(keychain))
        matches = re.findall(r'\b([0-9A-F]{40})\s+"(' + re.escape(prefix) + r'[^"\n]+)"', identities)
        if len(matches) != 1:
            raise SystemExit('Expected exactly one valid distribution identity of the required certificate class')
        # productbuild accepts the common name; codesign uses the unambiguous SHA-1.
        exported[output_name] = matches[0][1] if 'INSTALLER' in output_name else matches[0][0]
        if args.mode == 'developer-id' and matches[0][1] != os.environ['APPLE_SIGNING_IDENTITY']:
            raise SystemExit('Developer ID identity does not match repository configuration')
    run('security', 'set-key-partition-list', '-S', 'apple-tool:,apple:,codesign:', '-s', '-k', password, str(keychain))
    run('security', 'list-keychains', '-d', 'user', '-s', str(keychain))
    if args.mode == 'testflight':
        profile = stage / 'MusicX.provisionprofile'
        profile.write_bytes(base64.b64decode(os.environ['STORE_PROVISION_PROFILE'], validate=True))
        exported['STORE_PROFILE_PATH'] = str(profile)
    with Path(os.environ['GITHUB_ENV']).open('a') as output:
        for key, value in exported.items():
            output.write(f'{key}={value}\n')


if __name__ == '__main__':
    main()
