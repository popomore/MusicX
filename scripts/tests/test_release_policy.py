import copy
import importlib.util
from pathlib import Path
import unittest


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).resolve().parents[1] / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


POLICY = load('commit-policy')
SOURCE = load('release-source')
TESTFLIGHT = load('publish-testflight')


class CommitPolicyTests(unittest.TestCase):
    def test_conventional_headers_and_breaking_changes(self):
        for message in ('feat(player): support repeat', 'fix!: replace the queue API',
                        'perf: reduce allocations\n\nBREAKING CHANGE: configuration format changed',
                        'chore(release): promote main to stable'):
            POLICY.validate_message(message)
        for message in ('Add feature', 'feature: wrong type', 'feat: ', 'fix: 修复播放',
                        'feat: change\nbody without blank line', 'fix: change\n\nBREAKING CHANGE:'):
            with self.subTest(message=message), self.assertRaises(ValueError):
                POLICY.validate_message(message)

    def test_generated_merge_and_revert_require_the_generated_format(self):
        POLICY.validate_message('Merge pull request #1 from owner/branch', generated=True)
        POLICY.validate_message('Revert "fix: change"\n\nThis reverts commit ' + 'a' * 40 + '.', generated=True)
        with self.assertRaises(ValueError):
            POLICY.validate_message('Merge any random text', generated=True)
        with self.assertRaises(ValueError):
            POLICY.validate_message('Merge pull request #1 from owner/branch')

    def test_only_same_repo_release_sync_skips_historical_commits(self):
        pr = {'title': 'chore(release): promote main to stable',
              'head': {'ref': 'main', 'repo': {'full_name': 'owner/musicx'}},
              'base': {'ref': 'release', 'repo': {'full_name': 'owner/musicx'}}}
        commits = [{'sha': 'a' * 40, 'parents': [{}], 'commit': {'message': 'Legacy subject'}}]
        POLICY.validate_pr(pr, commits)
        for field, value in (('ref', 'feature'), ('repo', {'full_name': 'fork/musicx'})):
            changed = copy.deepcopy(pr)
            changed['head'][field] = value
            with self.assertRaises(ValueError):
                POLICY.validate_pr(changed, commits)


class ReleaseSourceTests(unittest.TestCase):
    def setUp(self):
        self.sha = 'a' * 40
        self.run = {'workflow_id': 123, 'path': '.github/workflows/ci-macos.yml', 'status': 'completed',
                    'event': 'push', 'conclusion': 'success', 'repository': {'full_name': 'owner/musicx'},
                    'head_repository': {'full_name': 'owner/musicx'}, 'head_branch': 'main', 'head_sha': self.sha}

    def test_rejects_forks_prs_failed_ci_other_workflows_and_wrong_dispatch_ref(self):
        SOURCE.validate(self.run, 123, 'owner/musicx', self.sha, 'main')
        for key, value in (('head_repository', {'full_name': 'fork/musicx'}), ('event', 'pull_request'),
                           ('conclusion', 'failure'), ('workflow_id', 456), ('path', 'ci.yml'),
                           ('head_branch', 'feature'), ('head_sha', 'main')):
            with self.subTest(key=key), self.assertRaises(ValueError):
                SOURCE.validate({**self.run, key: value}, 123, 'owner/musicx', self.sha)
        with self.assertRaises(ValueError):
            SOURCE.validate(self.run, 123, 'owner/musicx', self.sha, 'release')

    def test_stale_ci_only_enters_retry_recovery(self):
        with self.assertRaises(ValueError):
            SOURCE.validate(self.run, 123, 'owner/musicx', 'b' * 40)
        SOURCE.validate(self.run, 123, 'owner/musicx', 'b' * 40, retry=True)

    def test_relationship_urls_cannot_exfiltrate_apple_token(self):
        self.assertEqual(TESTFLIGHT.api_path('https://api.appstoreconnect.apple.com/v1/builds?a=b'), '/v1/builds?a=b')
        for url in ('https://evil.example/v1/builds', 'http://api.appstoreconnect.apple.com/v1/builds',
                    'https://api.appstoreconnect.apple.com@evil.example/v1/builds'):
            with self.subTest(url=url), self.assertRaises(ValueError):
                TESTFLIGHT.api_path(url)
