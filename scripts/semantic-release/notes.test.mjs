import { test } from 'node:test';
import assert from 'node:assert/strict';
import { generateNotes } from './macos.mjs';

const legacy = [
  { hash: 'e10bd714acb6226c2a80f042435273774a775adc',
    message: 'feat(macos): 优化设置分组与添加来源引导 (#12)\n\n* feat(macos): streamline settings layout' },
  { hash: '5dce5879181909b08fa23786e96ded18c7b0bb0c',
    message: 'feat(macos): 支持稳定版与全部版本更新 (#13)\n\n* feat(macos): add stable and preview update channels' },
];

function context(commits, branch = 'release') {
  return { commits, branch: { name: branch }, env: { RELEASE_MODE: 'plan' }, cwd: process.cwd(),
    options: { repositoryUrl: 'https://github.com/popomore/MusicX.git' },
    lastRelease: { gitTag: 'v1.0.0' }, nextRelease: { version: '1.1.0', gitTag: 'v1.1.0' } };
}

test('legacy subjects render in English with original commit and PR links', async () => {
  const commits = [...structuredClone(legacy), { hash: 'a'.repeat(40), message: 'fix(player): retain the queue' }];
  const original = structuredClone(commits);
  for (const branch of ['release', 'main']) {
    const notes = await generateNotes({}, context(commits, branch));
    assert.match(notes, /improve settings groups and source setup/);
    assert.match(notes, /support stable and preview updates/);
    assert.match(notes, /retain the queue/);
    assert.doesNotMatch(notes, /[\u3400-\u9fff\u3040-\u30ff\uac00-\ud7af]/);
    for (const commit of legacy) assert.ok(notes.includes(`/commit/${commit.hash}`));
    for (const number of [12, 13]) assert.ok(notes.includes(`/issues/${number}`));
    assert.deepEqual(commits, original);
  }
});

test('unrecognized non-English subjects stop planning rather than alter history', async () => {
  const commits = [{ ...legacy[0], hash: 'b'.repeat(40) }];
  await assert.rejects(generateNotes({}, context(commits)), /English release notes/);
});
