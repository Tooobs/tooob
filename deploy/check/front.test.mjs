// The Relay image's front.caddy, read as config: a Relay is checked live after each deploy, and the
// image's build has Caddy validate it, so these only make sure its rules are there. #99: a page's
// address saved outside the page ("Save link as") gets an explanation, not the shell, which would
// save as an empty page. The browser suite's stand-in Relay (bootstrap/check/harness.mjs) answers
// with these same texts, read from here.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const front = readFileSync(new URL('../relay/image/front.caddy', import.meta.url), 'utf8');

/** A top-level block, `<opening> {` to its closing `}`: its lines, comments dropped. */
function block(opening) {
  const m = new RegExp(`^${opening} \\{\\n([\\s\\S]*?)^\\}`, 'm').exec(front);
  assert.ok(m, `front.caddy has ${opening}`);
  return m[1];
}
const lines = (text) => text.split('\n').map((l) => l.trim()).filter((l) => l && !l.startsWith('#'));

/** What `<respond> <<TXT ... TXT 200` in `text` sends, its indentation stripped as Caddy does. */
function respondText(text, respond) {
  const m = new RegExp(`${respond} <<TXT\\n([\\s\\S]*?)\\n(\\t*)TXT 200`).exec(text);
  assert.ok(m, `${respond} answers with a heredoc and a 200`);
  return m[1].split('\n').map((l) => l.slice(m[2].length)).join('\n');
}

test("\"Save link as\" is told apart by its Fetch Metadata, and only at a page's address", () => {
  // Chromium (Chrome, Brave): Dest empty, Mode navigate, seen in the #99 experiment. Firefox: Dest
  // empty and the Fetch spec's default Mode, no-cors (its nsContextMenu's save channel). A page
  // visit is Dest document, and keeps getting the shell.
  assert.deepEqual(lines(block('@saved')), [
    'not path /__tooob/* /tooob-sw.js',
    'header Sec-Fetch-Dest empty',
    'header_regexp Sec-Fetch-Mode ^(navigate|no-cors)$',
  ]);
  const at = (s) => front.indexOf(s);
  assert.ok(at('handle @iroh {') < at('handle @saved {') && at('handle @saved {') < at('handle {'), "after iroh's paths, before the Bundle");
});

test('a saved file\'s address gets an explanation naming the file, a folder\'s naming its path, never cached', () => {
  const saved = block('handle @saved');
  assert.match(saved, /^\theader Cache-Control "no-store"$/m);
  assert.match(saved, /^\t@folder path \*\/$/m);
  const fix = 'tooob can only fetch it from inside the page:\nopen the link in your browser';
  assert.equal(respondText(saved, 'respond @folder'), `This isn't {http.request.uri.path}. ${fix}.`);
  assert.equal(respondText(saved, 'respond'), `This isn't {http.request.uri.path.file}. ${fix} and use ⬇ Download next to the file.`);
});

test("every answer of the Bundle's varies by Fetch Metadata, so a cached shell never answers a save", () => {
  assert.match(block('handle'), /^\theader Vary "Sec-Fetch-Dest, Sec-Fetch-Mode"$/m);
});
