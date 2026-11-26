import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { maintenanceBranch } from '../../release.config.mjs';

export const assetNames = ['MusicX-macos-arm64.dmg', 'MusicX-macos-arm64.dmg.sha256',
  'appcast.xml', 'appcast-all.xml', 'appcast-all.base'];
export const digest = bytes => createHash('sha256').update(bytes).digest('hex');
export const readJSON = file => JSON.parse(readFileSync(file, 'utf8'));
export const writeJSON = (file, value) => writeFileSync(file, JSON.stringify(value, null, 2) + '\n');
export const git = (...args) => execFileSync('git', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();

export function signingMode(plan) {
  if (Object.hasOwn(plan, 'signingMode')) {
    if (plan.signingMode !== 'developer-id') throw new Error('New release plans require Developer ID signing');
    return plan.signingMode;
  }
  // Historical plans recover their original artifacts without changing signatures.
  return plan.channel === 'stable' ? 'developer-id' : 'ad-hoc';
}

export function route(branch) {
  if (branch === 'main') return { channel: 'preview', feedTag: 'update-feed', makeLatest: false };
  if (branch === 'release') return { channel: 'stable', feedTag: 'update-feed', makeLatest: true };
  if (maintenanceBranch.test(branch)) return { channel: 'stable', feedTag: `update-feed-${branch}`, makeLatest: false };
  throw new Error('Not a configured release branch');
}

export function validatePlan(plan, expected = {}) {
  const routing = route(plan.branch);
  if (plan.schema !== 1 || !/^[0-9a-f]{40}$/.test(plan.sourceSha)
      || !/^[1-9]\d*$/.test(String(plan.runId)) || !/^[1-9]\d*$/.test(String(plan.ciRunId))
      || !/^[1-9]\d{0,8}$/.test(String(plan.runNumber))) throw new Error('Invalid release plan provenance');
  for (const [key, value] of Object.entries(expected)) {
    if (String(plan[key]) !== String(value)) throw new Error(`Release plan mismatch: ${key}`);
  }
  if (plan.release === false) return plan;
  signingMode(plan);
  const numeric = '(0|[1-9][0-9]*)';
  const match = new RegExp(`^(${numeric}\\.${numeric}\\.${numeric})(?:-preview\\.([1-9][0-9]*))?$`).exec(plan.version);
  if (!match || plan.tag !== `v${plan.version}` || plan.shortVersion !== match[1]
      || Boolean(match[5]) !== (routing.channel === 'preview')
      || plan.bundleVersion !== String(1000000 + Number(plan.runNumber))
      || Object.entries(routing).some(([key, value]) => plan[key] !== value)
      || typeof plan.notes !== 'string' || !plan.notes.trim()) throw new Error('Invalid release plan metadata');
  return plan;
}

export function validateAssets(plan, directory) {
  const receipt = readJSON(resolve(directory, 'validated-artifacts.json'));
  if (receipt.schema !== 1 || receipt.planSha256 !== digest(readFileSync(process.env.RELEASE_PLAN))
      || receipt.sourceSha !== plan.sourceSha || receipt.tag !== plan.tag
      || receipt.signingMode !== signingMode(plan)
      || Object.keys(receipt.files).sort().join('\n') !== [...assetNames].sort().join('\n')) {
    throw new Error('Artifacts do not match the immutable release plan');
  }
  for (const name of assetNames) {
    if (digest(readFileSync(resolve(directory, name))) !== receipt.files[name]) throw new Error(`Artifact digest mismatch: ${name}`);
  }
  return receipt;
}
