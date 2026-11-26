// The sole publisher adapter. No npm publication or issue/PR comments.
import { generateNotes as conventionalNotes } from '@semantic-release/release-notes-generator';
import { execFileSync } from 'node:child_process';
import { resolve } from 'node:path';
import { writeFileSync } from 'node:fs';
import { git, readJSON, validatePlan, validateAssets, signingMode } from './state.mjs';

// Render pre-policy subjects in English without rewriting their commits or links.
const legacySubjects = new Map([
  ['e10bd714acb6226c2a80f042435273774a775adc', 'feat(macos): improve settings groups and source setup (#12)'],
  ['5dce5879181909b08fa23786e96ded18c7b0bb0c', 'feat(macos): support stable and preview updates (#13)'],
]);

function planned(context) {
  return validatePlan(readJSON(context.env.RELEASE_PLAN), {
    sourceSha: git('rev-parse', 'HEAD'), branch: context.branch.name,
    runId: context.env.GITHUB_RUN_ID, ciRunId: context.env.CI_RUN_ID,
    runNumber: context.env.GITHUB_RUN_NUMBER,
  });
}

export async function generateNotes(_config, context) {
  if (context.env.RELEASE_MODE === 'publish') return planned(context).notes;
  const commits = context.commits.map(commit => ({ ...commit,
    message: legacySubjects.has(commit.hash)
      ? commit.message.replace(/^[^\n]*/, legacySubjects.get(commit.hash)) : commit.message,
  }));
  const notes = await conventionalNotes({ preset: 'conventionalcommits' }, { ...context, commits });
  if (/[\u3400-\u9fff\u3040-\u30ff\uac00-\ud7af]/u.test(notes)) {
    throw new Error('English release notes require an explicit translation for a historical commit');
  }
  const preview = context.branch.name === 'main';
  return `${notes}\n\n---\n\nDownload the Apple Silicon DMG and drag MusicX to Applications. Requires macOS 14 or later.\n\n`
    + (preview
      ? 'Preview build: signed with Developer ID, notarized by Apple, and stapled. TestFlight Internal is distributed separately after Apple processing.\n'
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
    MUSICX_BUNDLE_VERSION: plan.bundleVersion, SIGNING_MODE: signingMode(plan),
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
