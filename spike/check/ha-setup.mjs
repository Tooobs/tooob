// One-time onboarding of the local Home Assistant test container (localhost:8123).
// Creates a throwaway user and saves its tokens to ha-tokens.json for check-ha.mjs.
import { randomBytes } from 'node:crypto';
import { writeFileSync } from 'node:fs';

const HA = 'http://127.0.0.1:8123';
const CLIENT_ID = 'http://localhost:8088/'; // the shell's origin, so the HA frontend accepts the tokens
const password = randomBytes(18).toString('base64url');

const post = async (path, body, token, form) => {
  const r = await fetch(HA + path, {
    method: 'POST',
    headers: { ...(token && { authorization: `Bearer ${token}` }), 'content-type': form ? 'application/x-www-form-urlencoded' : 'application/json' },
    body: form ? new URLSearchParams(body) : JSON.stringify(body),
  });
  if (!r.ok) throw new Error(`${path}: ${r.status} ${await r.text()}`);
  return r.json();
};

const { auth_code } = await post('/api/onboarding/users', { client_id: CLIENT_ID, name: 'tooob test', username: 'tooob', password, language: 'en' });
const tokens = await post('/auth/token', { grant_type: 'authorization_code', code: auth_code, client_id: CLIENT_ID }, null, true);
await post('/api/onboarding/core_config', {}, tokens.access_token);
await post('/api/onboarding/analytics', {}, tokens.access_token);
await post('/api/onboarding/integration', { client_id: CLIENT_ID, redirect_uri: CLIENT_ID }, tokens.access_token);
writeFileSync('ha-tokens.json', JSON.stringify({ ...tokens, username: 'tooob', password }, null, 2));
console.log('onboarded; tokens in ha-tokens.json');
