import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const script = fileURLToPath(new URL('../scripts/release-environment.sh', import.meta.url));

test('release selection requires a stable tag at the checked-out main commit', () => {
  const folder = mkdtempSync(join(tmpdir(), 'site-release-source-'));
  const git = (...args) => execFileSync('git', args, { cwd: folder, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
  let count = 0;
  function select(overrides = {}) {
    const output = join(folder, 'output-' + count++);
    const result = spawnSync('bash', [script], {
      cwd: folder,
      encoding: 'utf8',
      env: {
        ...process.env, EVENT: 'push', REF_TYPE: 'tag', REF_NAME: 'site-v0.1.0',
        INPUT_TAG: '', GITHUB_OUTPUT: output, ...overrides,
      },
    });
    return { ...result, output: result.status === 0 && overrides.EVENT !== 'pull_request' ? readFileSync(output, 'utf8').trim() : '' };
  }
  try {
    git('init', '-q');
    git('config', 'user.name', 'Site source fixture');
    git('config', 'user.email', 'site@example.invalid');
    git('config', 'commit.gpgsign', 'false');
    git('config', 'core.hooksPath', '/dev/null');
    git('commit', '--allow-empty', '-qm', 'Initial source');
    git('update-ref', 'refs/remotes/origin/main', git('rev-parse', 'HEAD'));
    git('-c', 'tag.gpgsign=false', 'tag', 'site-v0.1.0');
    git('-c', 'tag.gpgsign=false', 'tag', 'site-v00.1.0');

    const stable = select();
    assert.equal(stable.status, 0, stable.stderr);
    assert.equal(stable.output, 'target_environment=production');
    const manual = select({ EVENT: 'workflow_dispatch', REF_TYPE: 'branch', REF_NAME: 'main', INPUT_TAG: 'site-v0.1.0' });
    assert.equal(manual.status, 0, manual.stderr);
    assert.equal(manual.output, 'target_environment=production');
    const main = select({ REF_TYPE: 'branch', REF_NAME: 'main' });
    assert.equal(main.status, 0, main.stderr);
    assert.equal(main.output, 'target_environment=test');
    assert.equal(select({ EVENT: 'pull_request' }).status, 0);

    for (const tag of ['site-v0.1.0-preview.1', 'site-v00.1.0', 'v0.1.0', 'site-v0.1.1', '--help']) {
      assert.notEqual(select({ REF_NAME: tag }).status, 0, tag);
    }
    assert.notEqual(select({ EVENT: 'workflow_dispatch', INPUT_TAG: '' }).status, 0);
    assert.notEqual(select({ REF_TYPE: 'branch', REF_NAME: 'release' }).status, 0);

    git('commit', '--allow-empty', '-qm', 'Unmerged source');
    assert.notEqual(select().status, 0, 'An older tag must not authorize a different checkout');
    git('-c', 'tag.gpgsign=false', 'tag', 'site-v0.2.0');
    const unrelated = select({ REF_NAME: 'site-v0.2.0' });
    assert.notEqual(unrelated.status, 0);
    assert.match(unrelated.stderr, /reachable from main/);
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
});
