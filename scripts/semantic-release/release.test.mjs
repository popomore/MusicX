import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { PassThrough } from 'node:stream';
import semanticRelease from 'semantic-release';
import config from '../../release.config.mjs';
import { route, validatePlan, signingMode } from './state.mjs';

const root = fileURLToPath(new URL('../../', import.meta.url));
const plugin = name => fileURLToPath(import.meta.resolve(name));

function fixture(t) {
  const directory = mkdtempSync(resolve(tmpdir(), 'musicx-semantic-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const remote = resolve(directory, 'origin.git');
  const cwd = resolve(directory, 'work');
  const env = { PATH: process.env.PATH, HOME: process.env.HOME, CI: 'true', GITHUB_ACTIONS: 'true',
    GITHUB_EVENT_NAME: 'push', RELEASE_MODE: 'plan', GITHUB_REF: 'refs/heads/main',
    GIT_AUTHOR_NAME: 'Release fixture', GIT_AUTHOR_EMAIL: 'fixture@example.invalid',
    GIT_COMMITTER_NAME: 'Release fixture', GIT_COMMITTER_EMAIL: 'fixture@example.invalid',
    GIT_CONFIG_COUNT: '3', GIT_CONFIG_KEY_0: 'commit.gpgsign', GIT_CONFIG_VALUE_0: 'false',
    GIT_CONFIG_KEY_1: 'core.hooksPath', GIT_CONFIG_VALUE_1: '/dev/null',
    GIT_CONFIG_KEY_2: `url.${remote}.insteadOf`, GIT_CONFIG_VALUE_2: 'https://github.com/musicx-fixture/local.git' };
  const run = (dir, ...args) => execFileSync('git', args, { cwd: dir, env, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
  run(directory, 'init', '--bare', '--initial-branch=release', remote);
  run(directory, 'clone', remote, cwd);
  const git = (...args) => run(cwd, ...args);
  // Branch discovery in the upstream core uses the process environment, so keep
  // the URL rewrite local to this disposable checkout as well.
  git('config', `url.${remote}.insteadOf`, 'https://github.com/musicx-fixture/local.git');
  const commit = message => git('commit', '--allow-empty', '-m', message);
  writeFileSync(resolve(cwd, 'Cargo.toml'), '[package]\nname = "fixture"\nversion = "0.2.0"\n');
  git('add', '.'); commit('chore: initialize'); git('tag', 'v1.0.0');
  git('push', 'origin', 'release', '--tags');
  git('checkout', '-b', 'main'); git('push', '-u', 'origin', 'main');
  const release = async (branch = 'main', dryRun = false) => {
    git('push', 'origin', branch);
    const stdout = new PassThrough(); stdout.resume();
    return semanticRelease({ ...config, dryRun, repositoryUrl: 'https://github.com/musicx-fixture/local.git',
      plugins: [[plugin('@semantic-release/commit-analyzer'), { preset: 'conventionalcommits' }],
        resolve(root, 'scripts/semantic-release/fixture-publisher.mjs')] },
    { cwd, env: { ...env, GITHUB_REF: `refs/heads/${branch}` }, stdout, stderr: stdout });
  };
  return { cwd, git, commit, release };
}

test('preview increments, stable accumulates, and maintenance stays isolated', async t => {
  const { git, commit, release, cwd } = fixture(t);
  commit('feat(player): add gapless playback');
  const first = await release();
  assert.equal(first.nextRelease.version, '1.1.0-preview.1');
  assert.match(first.nextRelease.notes, /gapless playback/);
  assert.match(first.nextRelease.notes, /Preview build: signed with Developer ID, notarized by Apple, and stapled/);
  assert.doesNotMatch(first.nextRelease.notes, /ad hoc signed/);
  commit('fix(player): keep the queue after pause');
  const second = await release();
  assert.equal(second.nextRelease.version, '1.1.0-preview.2');
  assert.match(second.nextRelease.notes, /queue after pause/);
  assert.doesNotMatch(second.nextRelease.notes, /gapless playback/);
  commit('docs: explain keyboard shortcuts');
  assert.equal(await release(), false);
  git('checkout', 'release');
  git('merge', '--no-ff', 'main', '-m', 'chore(release): promote main to stable');
  const stable = await release('release');
  assert.equal(stable.nextRelease.version, '1.1.0');
  assert.match(stable.nextRelease.notes, /gapless playback/);
  assert.match(stable.nextRelease.notes, /queue after pause/);
  git('checkout', 'main');
  git('merge', '--no-ff', 'release', '-m', 'chore(release): synchronize the stable tag into main');
  const sync = await release();
  assert.ok(!sync || !sync.nextRelease);
  commit('feat(player): add radio playback');
  const nextCycle = await release();
  assert.equal(nextCycle.nextRelease.version, '1.2.0-preview.1');
  assert.match(nextCycle.nextRelease.notes, /radio playback/);
  assert.doesNotMatch(nextCycle.nextRelease.notes, /gapless playback/);
  git('checkout', '-b', '1.0.x', 'v1.0.0');
  commit('fix(player): repair playback on the maintenance line');
  const maintenance = await release('1.0.x');
  assert.equal(maintenance.nextRelease.version, '1.0.1');
  assert.equal(maintenance.nextRelease.channel, '1.0.x');
  assert.deepEqual(route('1.0.x'), { channel: 'stable', feedTag: 'update-feed-1.0.x', makeLatest: false });
  const cargo = execFileSync('git', ['show', 'HEAD:Cargo.toml'], { cwd, encoding: 'utf8' });
  assert.match(cargo, /version = "0.2.0"/);
});

test('breaking headers/footers bump major and dry-run does not tag', async t => {
  const { git, commit, release } = fixture(t);
  commit('feat(api)!: replace the playback contract');
  assert.equal((await release('main', true)).nextRelease.version, '2.0.0-preview.1');
  assert.equal(git('tag', '--list', '*preview*'), '');
  commit('fix(api): validate requests\n\nBREAKING CHANGE: clients must send a protocol version');
  assert.equal((await release('main', true)).nextRelease.version, '2.0.0-preview.1');
});

test('plan provenance, new preview tags and stable routes are strict', () => {
  const plan = { schema: 1, branch: 'main', sourceSha: 'a'.repeat(40), runId: '42', ciRunId: '41', runNumber: '7',
    release: true, version: '1.1.0-preview.1', tag: 'v1.1.0-preview.1', shortVersion: '1.1.0',
    bundleVersion: '1000007', notes: 'Notes', ...route('main') };
  assert.equal(validatePlan(plan), plan);
  for (const change of [{ tag: 'v1.1.0' }, { version: '1.1.0-preview.0' }, { bundleVersion: '7' },
    { sourceSha: 'bad' }, { makeLatest: true }, { feedTag: 'update-feed-1.x' }]) {
    assert.throws(() => validatePlan({ ...plan, ...change }));
  }
  assert.throws(() => validatePlan(plan, { sourceSha: 'b'.repeat(40) }));
  assert.throws(() => route('pull/1/merge'));
});

test('new preview plans require Apple signing while historical plans retain their original mode', () => {
  const plan = { schema: 1, branch: 'main', sourceSha: 'a'.repeat(40), runId: '42', ciRunId: '41', runNumber: '7',
    release: true, version: '1.1.0-preview.1', tag: 'v1.1.0-preview.1', shortVersion: '1.1.0',
    bundleVersion: '1000007', notes: 'Notes', signingMode: 'developer-id', ...route('main') };
  assert.equal(validatePlan(plan), plan);
  assert.equal(signingMode(plan), 'developer-id');
  const legacy = { ...plan };
  delete legacy.signingMode;
  assert.equal(signingMode(validatePlan(legacy)), 'ad-hoc');
  assert.throws(() => validatePlan({ ...plan, signingMode: 'ad-hoc' }));
  assert.throws(() => validatePlan({ ...plan, signingMode: 'invalid' }));
});
