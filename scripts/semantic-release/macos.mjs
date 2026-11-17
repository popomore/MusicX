// The sole publisher adapter. No npm publication or issue/PR comments.
import { generateNotes as conventionalNotes } from '@semantic-release/release-notes-generator';
import { execFileSync } from 'node:child_process';
import { resolve } from 'node:path';
import { writeFileSync } from 'node:fs';
import { git, readJSON, validatePlan, validateAssets } from './state.mjs';

function planned(context) {
  return validatePlan(readJSON(context.env.RELEASE_PLAN), {
    sourceSha: git('rev-parse', 'HEAD'), branch: context.branch.name,
    runId: context.env.GITHUB_RUN_ID, ciRunId: context.env.CI_RUN_ID,
    runNumber: context.env.GITHUB_RUN_NUMBER,
  });
}

export async function generateNotes(_config, context) {
  if (context.env.RELEASE_MODE === 'publish') return planned(context).notes;
  const notes = await conventionalNotes({ preset: 'conventionalcommits' }, context);
  const preview = context.branch.name === 'main';
  return `${notes}\n\n---\n\nDownload the Apple Silicon DMG and drag MusicX to Applications. Requires macOS 14 or later.\n\n`
    + (preview
      ? 'Preview build: ad hoc signed; macOS may block the first launch. TestFlight Internal is distributed separately after Apple processing.\n'
      : 'Signed with Developer ID, notarized by Apple, and stapled. Automatic updates use signed Sparkle metadata.\n');
}

export async function verifyRelease(_config, context) {
  if (context.env.RELEASE_MODE !== 'publish') return;
  const plan = planned(context);
  if (context.nextRelease.gitTag !== plan.tag || context.nextRelease.version !== plan.version
      || context.nextRelease.gitHead !== plan.sourceSha) throw new Error('The release changed after packaging; refusing to tag');
  validateAssets(plan, context.env.RELEASE_ARTIFACTS);
}

export function publishPlan(plan, directory, env = process.env) {
  validateAssets(plan, directory);
  const notesFile = resolve(directory, 'release-notes.md');
  writeFileSync(notesFile, plan.notes);
  const releaseEnv = { ...env, RELEASE_TAG: plan.tag, SOURCE_SHA: plan.sourceSha,
    MUSICX_BUNDLE_VERSION: plan.bundleVersion, SIGNING_MODE: plan.channel === 'stable' ? 'developer-id' : 'ad-hoc',
    RELEASE_NOTES_FILE: notesFile, RELEASE_MAKE_LATEST: String(plan.makeLatest), UPDATE_FEED_TAG: plan.feedTag };
  for (const script of ['publish-macos.sh', 'publish-update-feed.sh']) {
    execFileSync('bash', [resolve('scripts', script), resolve(directory, 'MusicX-macos-arm64.dmg')],
      { env: releaseEnv, stdio: 'inherit' });
  }
  return { name: 'MusicX macOS', url: `https://github.com/${env.GITHUB_REPOSITORY}/releases/tag/${plan.tag}` };
}

export async function publish(_config, context) {
  return publishPlan(planned(context), context.env.RELEASE_ARTIFACTS, context.env);
}
