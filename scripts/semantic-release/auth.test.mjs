import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import { semanticEnvironment } from './auth.mjs';

// Exercise the pinned engine's real Git credential selection and push check.
const require = createRequire(import.meta.resolve('semantic-release'));
const core = pathToFileURL(require.resolve('@semantic-release/core'));
const { default: getGitAuthUrl } = await import(new URL('./lib/get-git-auth-url.js', core));
const { verifyAuth } = await import(new URL('./lib/git.js', core));

test('Actions installation tokens authenticate as x-access-token, including the GH_TOKEN workflow input', async t => {
  const directory = mkdtempSync(resolve(tmpdir(), 'musicx-git-auth-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  writeFileSync(resolve(directory, 'git'), `#!/usr/bin/env node
const value = process.argv.find(arg => arg.startsWith('https://'));
const url = new URL(value);
if (url.username !== 'x-access-token' || url.password !== 'fixture-installation-token') {
  process.stderr.write('Invalid installation token encoding');
  process.exit(128);
}
`, { mode: 0o755 });
  const original = { PATH: directory + ':' + process.env.PATH, GITHUB_ACTIONS: 'true', GITHUB_ACTION: '__run',
    GH_TOKEN: 'fixture-installation-token', GIT_TERMINAL_PROMPT: '0' };
  const context = env => ({ cwd: directory, env, branch: { name: 'main' },
    options: { repositoryUrl: 'https://github.com/fixture/musicx.git' } });
  const wrong = await getGitAuthUrl(context(original));
  await assert.rejects(verifyAuth(wrong, 'main', { cwd: directory, env: original }), /Invalid installation token encoding/);
  const env = semanticEnvironment(original);
  const url = await getGitAuthUrl(context(env));
  await verifyAuth(url, 'main', { cwd: directory, env });
});
