import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { delimiter, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const script = process.env.SITE_DEPLOY_SCRIPT ?? fileURLToPath(new URL('../scripts/deploy.sh', import.meta.url));

test('serialized deployment skips obsolete main builds and preserves tagged releases', () => {
  const folder = mkdtempSync(join(tmpdir(), 'site-current-deployment-'));
  const checkout = join(folder, 'checkout');
  const remote = join(folder, 'remote.git');
  const bin = join(folder, 'bin');
  const capture = join(folder, 'published');
  const git = (...args) => execFileSync('git', args, { cwd: checkout, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
  const deploy = (sha, overrides = {}) => {
    rmSync(capture, { force: true });
    const result = spawnSync('bash', [script], {
      cwd: checkout,
      encoding: 'utf8',
      env: {
        ...process.env, PATH: bin + delimiter + process.env.PATH, DEPLOY_CAPTURE: capture,
        EVENT: 'push', REF_TYPE: 'branch', REF_NAME: 'main', COMMIT_SHA: sha,
        TARGET_ENVIRONMENT: 'test', CLOUDFLARE_API_TOKEN: 'fixture-token', CLOUDFLARE_ACCOUNT_ID: 'fixture-account',
        ...overrides,
      },
    });
    return { ...result, published: existsSync(capture) ? readFileSync(capture, 'utf8').trim() : null };
  };
  try {
    mkdirSync(checkout);
    mkdirSync(bin);
    writeFileSync(join(bin, 'npm'), '#!/bin/sh\nprintf "%s\\n" "$*" >> "$DEPLOY_CAPTURE"\n', { mode: 0o755 });
    execFileSync('git', ['init', '--bare', '-q', remote], { stdio: 'ignore' });
    git('init', '-q');
    git('config', 'user.name', 'Deployment fixture');
    git('config', 'user.email', 'deploy@example.invalid');
    git('config', 'commit.gpgsign', 'false');
    git('config', 'core.hooksPath', '/dev/null');
    git('commit', '--allow-empty', '-qm', 'Older source');
    const older = git('rev-parse', 'HEAD');
    git('commit', '--allow-empty', '-qm', 'Current source');
    const current = git('rev-parse', 'HEAD');
    git('remote', 'add', 'origin', remote);
    git('push', 'origin', 'HEAD:refs/heads/main');

    git('checkout', '--detach', older);
    const obsolete = deploy(older);
    assert.equal(obsolete.status, 0, obsolete.stderr);
    assert.equal(obsolete.published, null, 'An older run must never overwrite the current main build');

    git('checkout', '--detach', current);
    const latest = deploy(current);
    assert.equal(latest.status, 0, latest.stderr);
    assert.equal(latest.published, 'exec -- wrangler deploy --env test');

    git('checkout', '--detach', older);
    const tagged = deploy(older, { REF_TYPE: 'tag', REF_NAME: 'site-v0.1.0', TARGET_ENVIRONMENT: 'production' });
    assert.equal(tagged.status, 0, tagged.stderr);
    assert.equal(tagged.published, 'exec -- wrangler deploy --env production');

    git('remote', 'set-url', 'origin', join(folder, 'missing.git'));
    const unavailable = deploy(older);
    assert.notEqual(unavailable.status, 0, 'An unavailable main ref must fail before publishing');
    assert.equal(unavailable.published, null);
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
});
