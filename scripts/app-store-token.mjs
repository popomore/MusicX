// App Store Connect team JWTs use ES256 and JOSE's fixed-width R || S encoding.
import { createPrivateKey, sign } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export function appStoreToken(pem, keyId, issuerId, now = Math.floor(Date.now() / 1000)) {
  if (!keyId || !issuerId || !Number.isSafeInteger(now) || now <= 0) throw new Error('Valid App Store team credentials are required');
  const key = createPrivateKey(pem);
  if (key.asymmetricKeyType !== 'ec' || key.asymmetricKeyDetails?.namedCurve !== 'prime256v1') {
    throw new Error('App Store Connect requires a P-256 private key');
  }
  const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url');
  const input = encode({ alg: 'ES256', kid: keyId, typ: 'JWT' }) + '.'
    + encode({ iss: issuerId, iat: now, exp: now + 600, aud: 'appstoreconnect-v1' });
  const signature = sign('sha256', Buffer.from(input), { key, dsaEncoding: 'ieee-p1363' });
  return input + '.' + signature.toString('base64url');
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  // The Python caller captures stdout and masks the JWT before using it in CI.
  process.stdout.write(appStoreToken(readFileSync(process.argv[2]), process.env.APPLE_API_KEY, process.env.APPLE_API_ISSUER));
}
