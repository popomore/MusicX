import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const source = fileURLToPath(new URL('../src/', import.meta.url));

test('translated UI copy stays in locale resources', () => {
  const violations = readdirSync(source, { recursive: true })
    .filter((name) => /\.(ts|astro)$/.test(name) && !name.startsWith('locales/'))
    .flatMap((name) => readFileSync(join(source, name), 'utf8').split('\n')
      .flatMap((line, index) => /\p{Script=Han}/u.test(line) ? [name + ':' + (index + 1)] : []));
  assert.deepEqual(violations, [], 'Move translated UI strings out of rendering and configuration code');
});
