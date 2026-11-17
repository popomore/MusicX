#!/usr/bin/env python3
"""Validate PR metadata as data. Run this file from the trusted base revision."""
import json
from pathlib import Path
import re
import sys

HEADER = re.compile(r'^(feat|fix|perf|refactor|docs|test|build|ci|chore|style|revert)(?:\([a-z0-9][a-z0-9._/-]*\))?!?: ([^\r\n]+)$')
MERGE = re.compile(r'^Merge (?:pull request #\d+ from \S+|branch .+|remote-tracking branch .+)(?:\n|$)')
REVERT = re.compile(r'^Revert "[^\n]+"\n\nThis reverts commit [0-9a-f]{40}\.\s*$')


def validate_message(message, generated=False):
    if generated and (MERGE.match(message) or REVERT.fullmatch(message)):
        return
    lines = message.splitlines()
    match = HEADER.fullmatch(lines[0] if lines else '')
    if not match or not match[2].strip() or len(lines[0]) > 100:
        raise ValueError('Use type(scope): English subject (at most 100 characters), with optional ! for breaking changes')
    if len(lines) > 1 and lines[1]:
        raise ValueError('Separate the subject and body with a blank line')
    if re.search(r'[\u3400-\u9fff\u3040-\u30ff\uac00-\ud7af]', message):
        raise ValueError('Commit subjects and bodies must be in English')
    for line in lines[1:]:
        if line.startswith(('BREAKING CHANGE:', 'BREAKING-CHANGE:')) and not line.split(':', 1)[1].strip():
            raise ValueError('Describe the breaking change after the footer')


def validate_pr(pr, commits):
    validate_message(pr['title'])
    # Sync PRs carry commits already reviewed on a protected release branch.
    head, base = pr.get('head', {}), pr.get('base', {})
    is_release_branch = lambda name: name in ('main', 'release') or bool(re.fullmatch(r'\d+(?:\.\d+)?\.x', name or ''))
    if (head.get('repo', {}).get('full_name') == base.get('repo', {}).get('full_name')
            and head.get('repo', {}).get('full_name')
            and is_release_branch(head.get('ref')) and is_release_branch(base.get('ref'))):
        return
    # Do not lint historical ancestors or execute any PR checkout/hooks.
    for commit in commits:
        try:
            validate_message(commit['commit']['message'], generated=len(commit['parents']) > 1
                             or commit['commit']['message'].startswith('Revert "'))
        except ValueError as error:
            raise ValueError(f'Commit {commit["sha"][:12]}: {error}') from error


if __name__ == '__main__':
    try:
        validate_pr(json.loads(Path(sys.argv[1]).read_text()), json.loads(Path(sys.argv[2]).read_text()))
    except ValueError as error:
        raise SystemExit(str(error))
