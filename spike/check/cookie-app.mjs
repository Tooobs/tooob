// Tiny cookie-auth app (like Immich/Nextcloud logins) to test tooob's host-side cookie jar.
// usage: node cookie-app.mjs  -> http://127.0.0.1:8010
import { randomBytes } from 'node:crypto';
import { createServer } from 'node:http';

const sessions = new Map();
const sid = (req) => /(?:^|;\s*)sid=([^;]+)/.exec(req.headers.cookie || '')?.[1];

createServer((req, res) => {
  const user = sessions.get(sid(req));
  if (req.method === 'POST' && req.url === '/login') {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      const id = randomBytes(16).toString('hex');
      sessions.set(id, new URLSearchParams(body).get('user') || 'anon');
      res.writeHead(303, { location: '/', 'set-cookie': `sid=${id}; HttpOnly; Path=/; SameSite=Lax` }).end();
    });
  } else if (req.url === '/me') {
    res.writeHead(user ? 200 : 401, { 'content-type': 'application/json' }).end(JSON.stringify({ user: user ?? null }));
  } else {
    res.writeHead(200, { 'content-type': 'text/html' }).end(user
      ? `<!doctype html><title>app</title><p id=who>logged in as ${user}</p>`
      : `<!doctype html><title>app</title><form method=post action=/login><input name=user value=alice><button id=go>log in</button></form>`);
  }
}).listen(8010, '127.0.0.1', () => console.log('cookie app on http://127.0.0.1:8010'));
