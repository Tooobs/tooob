// The Relay image's precompress.sh (#113), run by its Dockerfile on the Bundle in /srv: a brotli
// and a gzip copy beside each file a browser is sent compressed, which front.caddy's file_server
// serves (`precompressed br gzip`). The originals stay as they are: they're the Release's bytes.
// Needs brotli and gzip on PATH, as the image's build stage has them.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { chmodSync, mkdtempSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { brotliDecompressSync, gunzipSync } from 'node:zlib';

const script = new URL('../relay/image/precompress.sh', import.meta.url).pathname;

function bundle(files) {
  const dir = mkdtempSync(join(tmpdir(), 'precompress-'));
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(dirname(join(dir, path)), { recursive: true });
    writeFileSync(join(dir, path), text);
  }
  return dir;
}
const list = (dir, sub = '') => readdirSync(join(dir, sub)).sort();

test("each compressible file gets a .br and a .gz that decode to it, and the original stays", () => {
  const text = 'the Bundle '.repeat(500);
  const files = {
    'index.html': text, 'tooob-sw.js': text, 'version.json': text,
    '__tooob/pkg/web.js': text, '__tooob/pkg/web_bg.wasm': text, '__tooob/style.css': text,
  };
  const dir = bundle(files);
  try {
    execFileSync(script, [dir]);
    for (const path of Object.keys(files)) {
      assert.equal(readFileSync(join(dir, path), 'utf8'), text, `${path} unchanged`);
      assert.equal(brotliDecompressSync(readFileSync(join(dir, `${path}.br`))).toString(), text, `${path}.br`);
      assert.equal(gunzipSync(readFileSync(join(dir, `${path}.gz`))).toString(), text, `${path}.gz`);
    }
  } finally {
    rmSync(dir, { recursive: true });
  }
});

test('a copy that brotli fails to make fails the script, so the image build stops', () => {
  const dir = bundle({ 'index.html': '<p>', 'bin/brotli': '#!/bin/sh\nexit 1\n' });
  try {
    chmodSync(join(dir, 'bin/brotli'), 0o755);
    const env = { ...process.env, PATH: `${join(dir, 'bin')}:${process.env.PATH}` };
    assert.throws(() => execFileSync(script, [dir], { env, stdio: 'ignore' }));
  } finally {
    rmSync(dir, { recursive: true });
  }
});

test('nothing else gets a copy, and running it again changes nothing', () => {
  const dir = bundle({ 'icon.png': 'png', '__tooob/pkg/web.d.ts': 'types', 'index.html': '<p>' });
  try {
    execFileSync(script, [dir]);
    execFileSync(script, [dir]);
    assert.deepEqual(list(dir), ['__tooob', 'icon.png', 'index.html', 'index.html.br', 'index.html.gz']);
    assert.deepEqual(list(dir, '__tooob/pkg'), ['web.d.ts']);
  } finally {
    rmSync(dir, { recursive: true });
  }
});
