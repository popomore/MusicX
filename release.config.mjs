export const maintenanceBranch = /^\d+(?:\.\d+)?\.x$/;

export default {
  branches: [
    '+([0-9])?(.{+([0-9]),x}).x',
    { name: 'release', channel: false },
    { name: 'main', channel: 'preview', prerelease: 'preview' },
  ],
  tagFormat: 'v${version}',
  plugins: [
    ['@semantic-release/commit-analyzer', { preset: 'conventionalcommits' }],
    './scripts/semantic-release/macos.mjs',
  ],
};
