#!/usr/bin/env python3
"""Package the native Rust app; enable background playback before final signing."""
from pathlib import Path
import argparse, os, plistlib, subprocess

root = Path(__file__).resolve().parent
args = argparse.ArgumentParser()
args.add_argument('--target', choices=['ios', 'ios-simulator'], default='ios-simulator')
target = args.parse_args().target
destination = root / '.nsh/out' / target
subprocess.run([os.environ.get('NSH_BIN', 'nsh'), 'package', '--target', target, '--out', str(destination)], cwd=root, check=True)
app = destination / 'MusicX.app'
path = app / 'Info.plist'
info = plistlib.loads(path.read_bytes())
info['UIBackgroundModes'] = ['audio']
info['CFBundleDevelopmentRegion'] = 'en'
info['CFBundleLocalizations'] = ['en', 'zh-Hans']
info.setdefault('NSAppTransportSecurity', {})['NSAllowsLocalNetworking'] = True
info['UISupportedInterfaceOrientations'] = ['UIInterfaceOrientationPortrait']
for key in ['NSMicrophoneUsageDescription', 'NSSpeechRecognitionUsageDescription']:
    info.pop(key, None)
path.write_bytes(plistlib.dumps(info))
if target == 'ios-simulator':
    subprocess.run(['codesign', '--force', '--sign', '-', str(app)], check=True)
    subprocess.run(['codesign', '--verify', '--strict', str(app)], check=True)
else:
    binary = str(app / info['CFBundleExecutable'])
    if subprocess.run(['codesign', '--display', binary], capture_output=True).returncode == 0:
        subprocess.run(['codesign', '--remove-signature', binary], check=True)
print(f'{target}: {app}')
