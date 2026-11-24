import { test } from 'node:test';
import assert from 'node:assert/strict';
import { generateKeyPairSync, verify } from 'node:crypto';
import { appStoreToken } from '../app-store-token.mjs';

test('team JWTs carry Apple claims and a verifiable fixed-width ES256 signature', () => {
  const { privateKey, publicKey } = generateKeyPairSync('ec', { namedCurve: 'prime256v1' });
  const token = appStoreToken(privateKey.export({ format: 'pem', type: 'pkcs8' }), 'TEST123456', 'issuer-fixture', 1000);
  const [header, payload, signature] = token.split('.');
  const decode = value => JSON.parse(Buffer.from(value, 'base64url'));
  assert.deepEqual(decode(header), { alg: 'ES256', kid: 'TEST123456', typ: 'JWT' });
  assert.deepEqual(decode(payload), { iss: 'issuer-fixture', iat: 1000, exp: 1600, aud: 'appstoreconnect-v1' });
  assert.equal(Buffer.from(signature, 'base64url').length, 64);
  assert.ok(verify('sha256', Buffer.from(header + '.' + payload), { key: publicKey, dsaEncoding: 'ieee-p1363' }, Buffer.from(signature, 'base64url')));
  assert.ok(!verify('sha256', Buffer.from(header + '.' + payload + 'changed'), { key: publicKey, dsaEncoding: 'ieee-p1363' }, Buffer.from(signature, 'base64url')));
});

test('wrong key types and missing team credentials are rejected before token output', () => {
  const { privateKey } = generateKeyPairSync('ed25519');
  const pem = privateKey.export({ format: 'pem', type: 'pkcs8' });
  assert.throws(() => appStoreToken(pem, 'TEST123456', 'issuer-fixture'), /P-256/);
  assert.throws(() => appStoreToken(pem, '', ''), /credentials/);
});
