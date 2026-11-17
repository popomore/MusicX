#!/usr/bin/env python3
"""Find an immutable artifact in this release run; API errors must not trigger a rebuild."""
import json
import os
from pathlib import Path
import subprocess
import sys

repo, run_id = os.environ['GITHUB_REPOSITORY'], os.environ['GITHUB_RUN_ID']
pages = json.loads(subprocess.check_output([
    'gh', 'api', '--paginate', '--slurp', f'repos/{repo}/actions/runs/{run_id}/artifacts?per_page=100',
], text=True))
matches = [artifact for page in pages for artifact in page['artifacts'] if artifact['name'] == sys.argv[1]]
if len(matches) > 1 or any(artifact['expired'] for artifact in matches):
    raise SystemExit('Expected one unexpired release artifact; do not rebuild an already tagged release')
with Path(os.environ['GITHUB_OUTPUT']).open('a') as output:
    output.write(f'exists={str(bool(matches)).lower()}\n')
    if matches:
        output.write(f'id={matches[0]["id"]}\n')
