#!/usr/bin/env python3
"""Admit only successful push CI from this repository's protected release branches."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess


def gh(*args):
    return json.loads(subprocess.check_output(['gh', 'api', *args], text=True))


def release_branch(branch):
    return branch in ('main', 'release') or bool(re.fullmatch(r'(0|[1-9]\d*)(?:\.(0|[1-9]\d*))?\.x', branch))


def validate(run, workflow_id, repository, tip, selected_branch=None, retry=False):
    branch, sha = run.get('head_branch', ''), run.get('head_sha', '')
    if (run.get('workflow_id') != workflow_id or run.get('path') != '.github/workflows/ci-macos.yml'
            or run.get('event') != 'push' or run.get('conclusion') != 'success'
            or run.get('status') != 'completed'
            or run.get('repository', {}).get('full_name') != repository
            or run.get('head_repository', {}).get('full_name') != repository
            or not release_branch(branch) or not re.fullmatch('[0-9a-f]{40}', sha)):
        raise ValueError('Release requires successful push CI from the official workflow and repository')
    if selected_branch is not None and selected_branch != branch:
        raise ValueError('The dispatch ref must match the tested branch')
    if sha != tip and not retry:
        raise ValueError('The tested SHA is no longer the branch tip')
    return branch, sha


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('ci_run_id')
    parser.add_argument('--dispatch', action='store_true')
    args = parser.parse_args()
    if not re.fullmatch(r'[1-9]\d*', args.ci_run_id):
        parser.error('A numeric CI run ID is required')
    repo = os.environ['GITHUB_REPOSITORY']
    run = gh(f'repos/{repo}/actions/runs/{args.ci_run_id}')
    workflow = gh(f'repos/{repo}/actions/workflows/ci-macos.yml')
    branch = run.get('head_branch', '')
    if not release_branch(branch):
        raise SystemExit('Not a release branch')
    tip = gh(f'repos/{repo}/git/ref/heads/{branch}')['object']['sha']
    branch, sha = validate(run, workflow['id'], repo, tip,
                           None if args.dispatch else os.environ['GITHUB_REF_NAME'],
                           retry=not args.dispatch and int(os.environ.get('GITHUB_RUN_ATTEMPT', '1')) > 1)
    if args.dispatch:
        subprocess.run(['gh', 'workflow', 'run', 'release-macos.yml', '--repo', repo,
                        '--ref', branch, '--field', 'ci_run_id=' + args.ci_run_id], check=True)
    else:
        with Path(os.environ['GITHUB_OUTPUT']).open('a') as output:
            output.write(f'branch={branch}\nsource-sha={sha}\n')


if __name__ == '__main__':
    main()
