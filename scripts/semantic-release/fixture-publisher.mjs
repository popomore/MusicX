// Local bare-repository tests exercise the real version engine and notes adapter.
export { generateNotes } from './macos.mjs';
export async function publish(_config, { nextRelease }) {
  return { name: 'Fixture', version: nextRelease.version };
}
