#!/usr/bin/env bash
# Cut a Release (#59): ./release.sh X.Y.Z, or X.Y.Z-rc.N for a test Release.
set -euo pipefail
cd "$(dirname "$0")"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

refuse() {
  echo "release.sh: $1" >&2
  exit 1
}

[[ $# -eq 1 && $1 =~ ^[0-9]+\.[0-9]+\.[0-9]+(-rc\.[0-9]+)?$ ]] || refuse "usage: ./release.sh X.Y.Z (or X.Y.Z-rc.N for a test Release)"
RELEASE=$1
[[ -z $(git status --porcelain) ]] || refuse "the working tree has uncommitted changes"
[[ $(git symbolic-ref --short HEAD) == main ]] || refuse "a Release is cut from main"
git fetch -q origin main
# Commits on main not yet on origin go out with the Release, after the same checks.
git merge-base --is-ancestor origin/main HEAD || refuse "main is behind origin/main: pull first"

# unreleased: what CHANGELOG.md's "## Unreleased" section says, blank lines and all.
unreleased() {
  awk '/^## /{ inside = ($0 == "## Unreleased"); next } inside' CHANGELOG.md
}
[[ -n $(unreleased | tr -d '[:space:]') ]] || refuse "CHANGELOG.md's Unreleased section is empty: say what this Release changes"

# protocol_version [REF]: the Host's Protocol Version at REF, or in the working tree.
protocol_version() {
  local pattern='s/^pub const PROTOCOL_VERSION: &str = "\(.*\)";$/\1/p'
  if [[ $# -eq 0 ]]; then sed -n "$pattern" host/src/wire.rs; else git show "$1:host/src/wire.rs" | sed -n "$pattern"; fi
}

# major_minor X.Y.Z: X.Y
major_minor() { echo "$1" | cut -d. -f1-2; }

# The Release before this one, by its tag on main, whether or not it was published.
PROTOCOL=$(protocol_version)
PREVIOUS_RELEASE=$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || true)
PREVIOUS_RELEASE=${PREVIOUS_RELEASE#v}
PREVIOUS_PROTOCOL=${PREVIOUS_RELEASE:+$(protocol_version "v$PREVIOUS_RELEASE")}
if [[ -n $PREVIOUS_RELEASE && $PROTOCOL != "$PREVIOUS_PROTOCOL" && $(major_minor "$RELEASE") == $(major_minor "$PREVIOUS_RELEASE") ]]; then
  next=$(major_minor "$PREVIOUS_RELEASE"); next="${next%.*}.$(( ${next#*.} + 1 )).0"
  refuse "the Protocol Version changes from $PREVIOUS_PROTOCOL to $PROTOCOL since $PREVIOUS_RELEASE, so this is a minor Release, $next, not a patch (ADR 0017)"
fi

# browser_suite NAME [VAR=VALUE...]: the browser suite, on Playwright in windows you can watch, with
# VAR=VALUE in its environment. The tests that failed (or never ran, in a run that ran out of time)
# rerun once, saying so; failing twice stops the Release. A run that failed before any test ran
# (another run was going, or its setup failed) stops the Release, since a rerun would run nothing.
browser_suite() {
  local name=$1
  shift
  echo "release.sh: $name"
  (cd bootstrap/check && env "$@" npx playwright test) && return
  [[ $(failed_tests) -gt 0 ]] || refuse "$name: it failed before any test ran (see above)"
  echo "release.sh: $name: rerunning the tests that failed, once"
  (cd bootstrap/check && env "$@" npx playwright test --last-failed) || refuse "$name: a test failed twice"
}

# failed_tests: how many tests the browser suite's last run says failed or never ran.
failed_tests() {
  node -e 'try { console.log(require(process.argv[1]).failedTests.length) } catch { console.log(0) }' \
    "$PWD/bootstrap/check/test-results/.last-run.json"
}

# published_release: the newest Release on main that's published on GitHub (a tag whose release
# workflow failed has none), or nothing.
published_release() {
  local tag
  for tag in $(git tag --merged HEAD --sort=-creatordate --list 'v[0-9]*'); do
    if gh release view "$tag" --json tagName >/dev/null 2>&1; then echo "${tag#v}" && return; fi
  done
}

# cross_release_check: the previous Release's Bootstrap Bundle against this Host, and this
# Bootstrap Bundle against the previous Release's tooob (ADR 0017).
cross_release_check() {
  local skip="release.sh: cross-Release check: skipped," published target dir="$TMP/published"
  [[ -n $PREVIOUS_RELEASE ]] || { echo "$skip the first Release" && return; }
  published=$(published_release)
  [[ -n $published ]] || { echo "$skip no earlier Release is published" && return; }
  # rc's are test Releases: compatibility is owed from a version's final Release on, not to its rc's.
  [[ $published != "${RELEASE%-rc.*}"-rc.* ]] ||
    { echo "$skip $published is a test Release of ${RELEASE%-rc.*}" && return; }
  [[ $(protocol_version "v$published") == "$PROTOCOL" ]] ||
    { echo "$skip the Protocol Version changes since $published" && return; }
  # Linux Releases are static musl builds, which run on any Linux.
  target=$(rustc -vV | sed -n 's/^host: //p')
  target=${target/-linux-gnu/-linux-musl}
  gh release view "v$published" --json assets --jq '.assets[].name' | grep -qx "tooob-$target.tar.xz" ||
    { echo "$skip $published has no tooob for this machine ($target)" && return; }
  echo "release.sh: cross-Release check: downloading $published"
  gh release download "v$published" --dir "$dir" --pattern "tooob-$target.tar.xz" --pattern tooob-bootstrap.tar.xz
  tar -xJf "$dir/tooob-$target.tar.xz" -C "$dir"
  tar -xJf "$dir/tooob-bootstrap.tar.xz" -C "$dir"
  browser_suite "cross-Release check: $published's Bootstrap Bundle, this Host" TOOOB_WWW="$dir/tooob-bootstrap"
  browser_suite "cross-Release check: this Bootstrap Bundle, $published's tooob" TOOOB_BIN="$dir/tooob-$target/tooob"
}

# set_cargo_version FILE: the first `version = "…"` line, its package's, says $RELEASE.
set_cargo_version() {
  awk -v v="$RELEASE" '!done && /^version = "/ { $0 = "version = \"" v "\""; done = 1 } 1' "$1" >"$1.new"
  mv "$1.new" "$1"
}

# stamp_changelog: Unreleased becomes this Release's section, under a fresh, empty Unreleased.
stamp_changelog() {
  local body notes relay
  relay=$(iroh-relay --version | awk '{ print $2 }')
  body=$(unreleased | sed '/./,$!d')
  notes="Protocol Version: \`$PROTOCOL\`. Tested with iroh-relay $relay."
  if [[ -n $PREVIOUS_RELEASE && $PROTOCOL != "$PREVIOUS_PROTOCOL" ]]; then
    notes="**This Release changes the Protocol Version, from \`$PREVIOUS_PROTOCOL\` to \`$PROTOCOL\`: a Host and its Bootstrap must be upgraded together (ADR 0017).**"$'\n\n'"$notes"
  fi
  {
    awk '/^## Unreleased$/ { exit } 1' CHANGELOG.md
    printf '## Unreleased\n\n## %s — %s\n\n%s\n\n%s\n' "$RELEASE" "$(date -u +%Y-%m-%d)" "$notes" "$body"
    awk '/^## / { if (passed) on = 1; if ($0 == "## Unreleased") passed = 1 } on' CHANGELOG.md | sed '1s/^/\n/'
  } >CHANGELOG.md.new
  mv CHANGELOG.md.new CHANGELOG.md
}

git rev-parse -q --verify "refs/tags/v$RELEASE" >/dev/null && refuse "v$RELEASE is already tagged"
command -v iroh-relay >/dev/null || refuse "needs iroh-relay: cargo install iroh-relay --features server --version 1.3.0"
[[ -x bootstrap/check/node_modules/.bin/playwright ]] ||
  refuse "needs Playwright: cd bootstrap/check && npm ci && npx playwright install chromium"
chromium=$(cd bootstrap/check && node -e 'import("@playwright/test").then(({ chromium }) => console.log(chromium.executablePath()))')
[[ -x $chromium ]] || refuse "needs Playwright's Chromium: cd bootstrap/check && npx playwright install chromium"

echo "release.sh: ./check.sh"
./check.sh || refuse "./check.sh failed"
browser_suite "the browser suite"
cross_release_check

set_cargo_version host/Cargo.toml
set_cargo_version bootstrap/web/Cargo.toml
cargo update -q --workspace --offline
(cd bootstrap && cargo update -q --workspace --offline)
# A test Release's notes are "Unreleased", which stays for the Release it's testing.
[[ $RELEASE == *-rc.* ]] || stamp_changelog

git add host/Cargo.toml bootstrap/web/Cargo.toml Cargo.lock bootstrap/Cargo.lock CHANGELOG.md
git commit -q -m "Release $RELEASE"
git tag -a "v$RELEASE" -m "Release $RELEASE"
# main first, then the tag on its own: pushed together, GitHub starts no workflow for the tag.
git push -q origin HEAD:main
git push -q origin "v$RELEASE" ||
  refuse "main is pushed, but its tag isn't: push it with git push origin v$RELEASE"

cat <<MSG
release.sh: pushed Release $RELEASE (tag v$RELEASE). The release workflow builds and publishes it:
  https://github.com/Tooobs/tooob/actions
It's the first place Linux is tested. If it fails, fix it on main and cut the next patch Release:
never move a pushed tag.
MSG
