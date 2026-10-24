#!/usr/bin/env bash
# Cut a Release (#59): ./release.sh X.Y.Z
set -euo pipefail
cd "$(dirname "$0")"

refuse() {
  echo "release.sh: $1" >&2
  exit 1
}

[[ $# -eq 1 && $1 =~ ^[0-9]+\.[0-9]+\.[0-9]+(-rc\.[0-9]+)?$ ]] || refuse "usage: ./release.sh X.Y.Z (or X.Y.Z-rc.N for a test Release)"
VERSION=$1
[[ -z $(git status --porcelain) ]] || refuse "the working tree has uncommitted changes"
[[ $(git symbolic-ref --short HEAD) == main ]] || refuse "a Release is cut from main"
git fetch -q origin main
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

# minor X.Y.Z: X.Y
minor() { echo "$1" | cut -d. -f1-2; }

PROTOCOL=$(protocol_version)
PREVIOUS=$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || true)
PREVIOUS_PROTOCOL=${PREVIOUS:+$(protocol_version "$PREVIOUS")}
if [[ -n $PREVIOUS && $PROTOCOL != "$PREVIOUS_PROTOCOL" && $(minor "$VERSION") == $(minor "${PREVIOUS#v}") ]]; then
  next=$(minor "${PREVIOUS#v}"); next="${next%.*}.$(( ${next#*.} + 1 )).0"
  refuse "the Protocol Version changes from $PREVIOUS_PROTOCOL to $PROTOCOL since $PREVIOUS, so this is a minor Release, $next, not a patch (ADR 0017)"
fi

# browser_suite NAME [VAR=VALUE...]: the browser suite with VAR=VALUE in its environment. A failed
# test reruns once, saying so; failing twice stops the Release.
browser_suite() {
  local name=$1 state
  shift
  state=$(mktemp -d)/rerun.json
  echo "release.sh: $name"
  (cd bootstrap/check && env "$@" node --test --test-rerun-failures="$state") && return
  echo "release.sh: $name: rerunning the tests that failed, once"
  (cd bootstrap/check && env "$@" node --test --test-rerun-failures="$state") ||
    refuse "$name: a test failed twice"
}

# cross_release_check: the previous Release's Bootstrap Bundle against this Host, and this
# Bootstrap Bundle against the previous Release's tooob (ADR 0017).
cross_release_check() {
  if [[ -z $PREVIOUS ]]; then
    echo "release.sh: cross-Release check: skipped, the first Release"
    return
  elif [[ $PROTOCOL != "$PREVIOUS_PROTOCOL" ]]; then
    echo "release.sh: cross-Release check: skipped, the Protocol Version changes ($PREVIOUS_PROTOCOL to $PROTOCOL)"
    return
  fi
  local dir target
  dir=$(mktemp -d)
  target=$(rustc -vV | sed -n 's/^host: //p')
  echo "release.sh: cross-Release check: downloading $PREVIOUS"
  gh release download "$PREVIOUS" --dir "$dir" \
    --pattern "tooob-$target.tar.xz" --pattern tooob-bootstrap.tar.xz
  tar -xJf "$dir/tooob-$target.tar.xz" -C "$dir"
  tar -xJf "$dir/tooob-bootstrap.tar.xz" -C "$dir"
  browser_suite "cross-Release check: $PREVIOUS's Bootstrap Bundle, this Host" TOOOB_WWW="$dir/tooob-bootstrap"
  browser_suite "cross-Release check: this Bootstrap Bundle, $PREVIOUS's tooob" TOOOB_BIN="$dir/tooob-$target/tooob"
}

# set_version FILE: the first `version = "…"` line, its package's, says $VERSION.
set_version() {
  awk -v v="$VERSION" '!done && /^version = "/ { $0 = "version = \"" v "\""; done = 1 } 1' "$1" >"$1.new"
  mv "$1.new" "$1"
}

# stamp_changelog: Unreleased becomes this Release's section, under a fresh, empty Unreleased.
stamp_changelog() {
  local body notes relay
  relay=$(iroh-relay --version | awk '{ print $2 }')
  body=$(unreleased | sed '/./,$!d')
  notes="Protocol Version: \`$PROTOCOL\`. Tested with iroh-relay $relay."
  if [[ -n $PREVIOUS && $PROTOCOL != "$PREVIOUS_PROTOCOL" ]]; then
    notes="**This Release changes the Protocol Version, from \`$PREVIOUS_PROTOCOL\` to \`$PROTOCOL\`: a Host and its Bootstrap must be upgraded together (ADR 0017).**"$'\n\n'"$notes"
  fi
  {
    awk '/^## Unreleased$/ { exit } 1' CHANGELOG.md
    printf '## Unreleased\n\n## %s — %s\n\n%s\n\n%s\n' "$VERSION" "$(date -u +%Y-%m-%d)" "$notes" "$body"
    awk '/^## / { if (passed) on = 1; if ($0 == "## Unreleased") passed = 1 } on' CHANGELOG.md | sed '1s/^/\n/'
  } >CHANGELOG.md.new
  mv CHANGELOG.md.new CHANGELOG.md
}

git rev-parse -q --verify "refs/tags/v$VERSION" >/dev/null && refuse "v$VERSION is already tagged"
command -v iroh-relay >/dev/null || refuse "needs iroh-relay: cargo install iroh-relay --features server --version 1.3.0"

echo "release.sh: ./check.sh"
./check.sh || refuse "./check.sh failed"
browser_suite "the browser suite"
cross_release_check

set_version host/Cargo.toml
set_version bootstrap/web/Cargo.toml
cargo update -q --workspace --offline
(cd bootstrap && cargo update -q --workspace --offline)
# A test Release's notes are "Unreleased", which stays for the Release it's testing.
[[ $VERSION == *-rc.* ]] || stamp_changelog

git add host/Cargo.toml bootstrap/web/Cargo.toml Cargo.lock bootstrap/Cargo.lock CHANGELOG.md
git commit -q -m "Release $VERSION"
git tag -a "v$VERSION" -m "Release $VERSION"
git push -q --atomic origin HEAD:main "v$VERSION"

cat <<MSG
release.sh: pushed Release $VERSION (tag v$VERSION). The release workflow builds and publishes it:
  https://github.com/Tooobs/tooob/actions
It's the first place Linux is tested. If it fails, fix it on main and cut the next patch Release:
never move a pushed tag.
MSG
