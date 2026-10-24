#!/usr/bin/env node
// The Bootstrap Bundle's manifest, __tooob/version.json: its Release, its Protocol Version, and
// every other file's SHA-256. The Bundle is identified by the manifest's own hash (ADR 0017).
//   node manifest.mjs DIR           write DIR's manifest
//   node manifest.mjs --check DIR   say which files no longer match it, and fail if any
//   node manifest.mjs --hash DIR    print the Bundle's hash: its manifest's SHA-256
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const MANIFEST_PATH = '__tooob/version.json';
const USAGE = 'usage: node manifest.mjs [--check | --hash] DIR';

/** A refusal the person running this should read, not a bug: printed without a stack trace. */
class Refused extends Error {}

/** `pattern`'s first group in `file` (relative to bootstrap/), or die saying what's missing. */
function scrape(file, pattern, what) {
  const found = readFileSync(join(HERE, file), 'utf8').match(pattern);
  if (!found) throw new Refused(`no ${what} in bootstrap/${file}`);
  return found[1];
}

/** Every file under `dir` but dotfiles and the manifest, as sorted paths relative to `dir`. A
 *  symlink is refused: what it points to isn't the Bundle's to vouch for. */
function bundleFiles(dir, under = '') {
  return readdirSync(join(dir, under), { withFileTypes: true })
    .filter((e) => !e.name.startsWith('.'))
    .flatMap((e) => {
      const path = under ? `${under}/${e.name}` : e.name;
      if (e.isSymbolicLink()) throw new Refused(`${path}: a symlink, not a file`);
      return e.isDirectory() ? bundleFiles(dir, path) : path === MANIFEST_PATH ? [] : [path];
    })
    .sort();
}

const sha256 = (path) => createHash('sha256').update(readFileSync(path)).digest('hex');

/** The Release and Protocol Version this source builds: what a manifest must name. */
function versions() {
  return {
    release: scrape('web/Cargo.toml', /^version = "(.+)"\r?$/m, 'Release'),
    protocolVersion: scrape('web/src/lib.rs', /^const PROTOCOL_VERSION: &str = "(.+)";\r?$/m, 'Protocol Version'),
  };
}

function manifest(dir) {
  return {
    ...versions(),
    files: Object.fromEntries(bundleFiles(dir).map((f) => [f, sha256(join(dir, f))])),
  };
}

/** DIR's manifest, as a path that exists. */
function manifestIn(dir) {
  const path = join(dir, MANIFEST_PATH);
  if (!existsSync(path)) throw new Refused(`${dir}: no manifest (${MANIFEST_PATH}); run build.sh`);
  return path;
}

/** What's wrong with DIR against its manifest: a Release or Protocol Version this source doesn't
 *  build, and one line per file changed, missing or unlisted. */
function mismatches(dir) {
  const named = JSON.parse(readFileSync(manifestIn(dir), 'utf8'));
  const listed = named.files;
  const present = new Set(bundleFiles(dir));
  return [
    ...Object.entries(versions())
      .filter(([key, built]) => named[key] !== built)
      .map(([key, built]) => `${key}: ${named[key]}, but this source builds ${built}`),
    ...Object.entries(listed).flatMap(([f, hash]) =>
      !present.has(f) ? [`${f}: missing`] : sha256(join(dir, f)) !== hash ? [`${f}: changed`] : [],
    ),
    ...[...present].filter((f) => !(f in listed)).map((f) => `${f}: not in the manifest`),
  ];
}

function main([first, dir]) {
  if ((first === '--check' || first === '--hash') && !dir) throw new Refused(USAGE);
  if (first === '--hash') {
    console.log(sha256(manifestIn(dir)));
    return 0;
  }
  if (first === '--check') {
    const problems = mismatches(dir);
    for (const p of problems) console.error(`manifest.mjs: ${p}`);
    return problems.length ? 1 : 0;
  }
  if (!first || first.startsWith('-')) throw new Refused(USAGE);
  writeFileSync(join(first, MANIFEST_PATH), JSON.stringify(manifest(first), null, 2) + '\n');
  return 0;
}

try {
  process.exitCode = main(process.argv.slice(2));
} catch (e) {
  if (!(e instanceof Refused)) throw e;
  console.error(`manifest.mjs: ${e.message}`);
  process.exitCode = 1;
}
