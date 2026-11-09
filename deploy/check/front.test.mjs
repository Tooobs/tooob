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

test("/__tooob/relay.json is tooob-relay's answer, and the Bundle's version.json stays the Bundle's", () => {
  // #128, ADR 0030: open or enrolled, from the listener start.sh gives tooob-relay.
  assert.deepEqual(lines(block('handle /__tooob/relay.json')), ['reverse_proxy 127.0.0.1:3341']);
  const start = readFileSync(new URL('../relay/image/start.sh', import.meta.url), 'utf8');
  assert.match(start, /^tooob-relay .*--status-addr 127\.0\.0\.1:3341 &$/m);
  assert.ok(!lines(front).some((l) => l.includes('version.json')), 'version.json is left to the Bundle handler');
});

test('/__tooob/enrolled/<key> is tooob-relay\'s answer, before the Bundle', () => {
  // #130: a Host asks whether its Relay carries it.
  assert.deepEqual(lines(block('handle /__tooob/enrolled/\\*')), ['reverse_proxy 127.0.0.1:3341']);
  assert.ok(front.indexOf('handle /__tooob/enrolled/*') < front.indexOf('handle {'), 'before the Bundle handler');
});

test('/__tooob/api/* (the enroll API) is tooob-relay\'s, before the Bundle', () => {
  // #140, ADR 0031: a Sign-up Page's PUT and DELETE, each with an Operator Token.
  assert.deepEqual(lines(block('handle /__tooob/api/\\*')), ['reverse_proxy 127.0.0.1:3341']);
  assert.ok(front.indexOf('handle /__tooob/api/*') < front.indexOf('handle @saved {'), 'before the saved-page and Bundle handlers');
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

test("the Bundle's files and its shell are sent as the image's brotli or gzip copies, nothing else is", () => {
  // #113: the .br and .gz files the image build writes (precompress.sh); file_server adds
  // Accept-Encoding to the Vary above. iroh's paths and the saved-page texts stay as they are.
  assert.match(block('handle'), /^\tfile_server \{\n\t\tprecompressed br gzip\n\t\}$/m);
  assert.equal(front.match(/precompressed/g).length, 1, 'only in the Bundle handler');
  assert.doesNotMatch(front, /^\s*encode\b/m, 'nothing compressed on the fly');
});
