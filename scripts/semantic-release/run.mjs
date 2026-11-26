#!/usr/bin/env node
import semanticRelease from 'semantic-release';
import config from '../../release.config.mjs';
import { appendFileSync, existsSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { execFileSync } from 'node:child_process';
import { git, readJSON, writeJSON, validatePlan, validateAssets, route } from './state.mjs';
import { publishPlan } from './macos.mjs';
import { semanticEnvironment } from './auth.mjs';

const mode = process.argv[2];
if (!['plan', 'publish'].includes(mode)) throw new Error('Use plan or publish');
const env = semanticEnvironment({ ...process.env, RELEASE_MODE: mode });
const planFile = env.RELEASE_PLAN = resolve(env.RELEASE_PLAN || 'target/release-plan/plan.json');
const expected = { branch: env.RELEASE_BRANCH, sourceSha: env.SOURCE_SHA,
  runId: env.GITHUB_RUN_ID, ciRunId: env.CI_RUN_ID, runNumber: env.GITHUB_RUN_NUMBER };
if (git('rev-parse', 'HEAD') !== expected.sourceSha) throw new Error('Checkout is not the tested source');
const repositoryUrl = `https://github.com/${env.GITHUB_REPOSITORY}.git`;
const options = { ...config, repositoryUrl, dryRun: mode === 'plan', ci: true };
// workflow_dispatch has the tested branch ref. Never infer a branch from PR metadata.
if (env.GITHUB_REF !== `refs/heads/${expected.branch}`) throw new Error('Wrong dispatch branch');

function remoteTag(tag) {
  const output = git('ls-remote', '--tags', 'origin', `refs/tags/${tag}`, `refs/tags/${tag}^{}`);
  if (!output) return null;
  const lines = output.split('\n');
  return (lines.find(line => line.endsWith('^{}')) || lines[0]).split(/\s/)[0];
}

function assertCurrent(plan, recovering = false) {
  const tip = git('ls-remote', 'origin', `refs/heads/${plan.branch}`).split(/\s/)[0];
  if (tip !== plan.sourceSha && !recovering) throw new Error('Branch advanced after CI; wait for the newer push');
}

// If the tag was pushed before a crash, repair only its missing channel note and
// resume publication from this run's byte-identical artifacts. Never retag/rebuild.
function recover(plan) {
  validateAssets(plan, env.RELEASE_ARTIFACTS);
  const ref = `refs/notes/semantic-release-${plan.tag}`;
  const authEnv = { ...env, GIT_AUTHOR_NAME: 'semantic-release-bot', GIT_AUTHOR_EMAIL: 'semantic-release-bot@martynus.net',
    GIT_COMMITTER_NAME: 'semantic-release-bot', GIT_COMMITTER_EMAIL: 'semantic-release-bot@martynus.net',
    GIT_CONFIG_COUNT: '1', GIT_CONFIG_KEY_0: 'http.https://github.com/.extraheader',
    GIT_CONFIG_VALUE_0: `AUTHORIZATION: basic ${Buffer.from(`x-access-token:${env.GITHUB_TOKEN || env.GH_TOKEN}`).toString('base64')}` };
  const command = args => {
    try { return execFileSync('git', args, { env: authEnv, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim(); }
    catch { throw new Error('Release note recovery failed; credentials and remote output were suppressed'); }
  };
  const channel = plan.channel === 'preview' ? 'preview' : plan.branch === 'release' ? null : plan.branch;
  if (git('ls-remote', 'origin', ref)) {
    command(['fetch', 'origin', `${ref}:${ref}`]);
    const note = JSON.parse(command(['notes', '--ref', ref, 'show', plan.tag]));
    if (!note.channels?.includes(channel)) throw new Error('Existing tag channel note conflicts with the plan');
  } else {
    command(['notes', '--ref', ref, 'add', '-m', JSON.stringify({ channels: [channel] }), plan.tag]);
    command(['push', 'origin', ref]);
  }
  publishPlan(plan, env.RELEASE_ARTIFACTS, env);
}

let plan;
if (existsSync(planFile)) {
  plan = validatePlan(readJSON(planFile), expected);
} else if (mode === 'plan') {
  assertCurrent({ ...expected });
  const result = await semanticRelease(options, { env });
  const next = result && result.nextRelease;
  plan = { schema: 1, ...expected, release: Boolean(next) };
  if (next) Object.assign(plan, route(plan.branch), { version: next.version, tag: next.gitTag,
    shortVersion: next.version.split('-')[0], bundleVersion: String(1000000 + Number(env.GITHUB_RUN_NUMBER)),
    signingMode: 'developer-id', notes: next.notes });
  validatePlan(plan, expected);
  mkdirSync(dirname(planFile), { recursive: true });
  writeJSON(planFile, plan);
} else throw new Error('Publication requires the original release plan artifact');

if (mode === 'publish') {
  if (!plan.release) throw new Error('No planned release');
  // Validate before core can add a channel or create a tag.
  validateAssets(plan, env.RELEASE_ARTIFACTS);
  const existing = remoteTag(plan.tag);
  if (existing && existing !== plan.sourceSha) throw new Error('Existing tag points to another source; approval is required to move tags');
  assertCurrent(plan, Boolean(existing));
  if (existing) recover(plan);
  else {
    const result = await semanticRelease(options, { env });
    if (!result?.nextRelease || result.nextRelease.gitTag !== plan.tag) throw new Error('The planned release was not published');
  }
}
if (env.GITHUB_OUTPUT) {
  appendFileSync(env.GITHUB_OUTPUT, `release=${plan.release}\n`);
  if (plan.release) for (const name of ['tag', 'version', 'shortVersion', 'bundleVersion', 'channel', 'feedTag']) {
    appendFileSync(env.GITHUB_OUTPUT, `${name}=${plan[name]}\n`);
  }
}
