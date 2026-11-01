// The Relay image's front.caddy, read as config: a Relay is checked live after each deploy, so these
// only make sure its rules are there. #99: a page's address saved outside the page ("Save link
// as") gets an explanation, not the shell, which would save as an empty page.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const front = readFileSync(new URL('../relay/image/front.caddy', import.meta.url), 'utf8');

/** Named matcher `@name { ... }`'s lines, trimmed, comments dropped. */
function matcher(name) {
  const m = new RegExp(`^@${name} \\{\\n([^}]*)^\\}`, 'm').exec(front);
  assert.ok(m, `front.caddy has @${name}`);
  return m[1].split('\n').map((l) => l.trim()).filter((l) => l && !l.startsWith('#'));
}

/** `handle @name { ... }`'s body. */
function handle(name) {
  const m = new RegExp(`^handle @${name} \\{\\n([\\s\\S]*?)^\\}`, 'm').exec(front);
  assert.ok(m, `front.caddy has handle @${name}`);
  return m[1];
}

/** The body a `respond <<TXT ... TXT` in `block` sends, its indentation as Caddy strips it. */
function respondText(block) {
  const m = /respond <<TXT\n([\s\S]*?)\n(\t*)TXT 200/.exec(block);
  assert.ok(m, 'answers with a heredoc and a 200');
  return m[1].split('\n').map((l) => l.slice(m[2].length)).join('\n');
}

// What browsers send for "Save link as": Chromium (Chrome, Brave) Dest empty and Mode navigate, seen
// in the #99 experiment; Firefox Dest empty and the Fetch spec's default Mode, no-cors (its
// nsContextMenu's save channel). A page visit is Dest document, and keeps getting the shell.
const SAVED = ['header Sec-Fetch-Dest empty', 'header Sec-Fetch-Mode navigate', 'header Sec-Fetch-Mode no-cors'];

test('"Save link as" of a file\'s address gets an explanation naming the file, never cached', () => {
  const lines = matcher('saved');
  for (const line of SAVED) assert.ok(lines.includes(line), `@saved: ${line}`);
  assert.ok(lines.includes('not path /__tooob/* /tooob-sw.js */'), 'only at a page address, and not a folder');
  const block = handle('saved');
  assert.match(block, /header Cache-Control "no-store"/);
  assert.equal(
    respondText(block),
    "This isn't {http.request.uri.path.file}. tooob can only fetch it from inside the page:\n" +
      'open the link in your browser and use ⬇ Download next to the file.',
  );
});

test('"Save link as" of a folder\'s address says the same, naming its path', () => {
  const lines = matcher('saved_folder');
  for (const line of SAVED) assert.ok(lines.includes(line), `@saved_folder: ${line}`);
  assert.ok(lines.includes('path */'), 'a folder');
  assert.ok(lines.includes('not path /__tooob/*'), 'not a Bundle folder');
  const block = handle('saved_folder');
  assert.match(block, /header Cache-Control "no-store"/);
  assert.equal(
    respondText(block),
    "This isn't {http.request.uri.path}. tooob can only fetch it from inside the page:\n" +
      'open the link in your browser.',
  );
});

test('the explanations come before the Bundle, and after iroh\'s paths', () => {
  const at = (s) => front.indexOf(s);
  assert.ok(at('handle @iroh') < at('handle @saved {') && at('handle @saved {') < at('handle {'));
  assert.ok(at('handle @iroh') < at('handle @saved_folder {') && at('handle @saved_folder {') < at('handle {'));
});

test("a page's address varies by Fetch Metadata, so a cached shell never answers a save, nor the other way", () => {
  assert.match(front, /header @page Vary "Sec-Fetch-Dest, Sec-Fetch-Mode"/);
});
