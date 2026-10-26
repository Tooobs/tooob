#!/bin/sh
# The Bootstrap Bundle a Relay serves (ADR 0020, #67), into DIR/www: a Release's, checked against its
# checksum, or bootstrap/www as built here. Prints which, and the Bundle's hash to compare with its
# Release's notes. The Relay setups' deploy scripts run it.
#   ./bundle.sh DIR [RELEASE]   that Release's Bundle, e.g. 0.1.0 (default: the newest Release)
#   ./bundle.sh DIR --local     bootstrap/www, built first with bootstrap/build.sh
# TOOOB_RELEASES points at another repo's Releases (a fork's), as GitHub lays them out.
set -eu
RELEASES=${TOOOB_RELEASES:-https://github.com/Tooobs/tooob/releases}
[ $# -ge 1 ] || { echo "usage: bundle.sh DIR [RELEASE | --local]" >&2 && exit 1; }
dir=$1 release=${2:-}
[ ! -e "$dir/www" ] || { echo "bundle.sh: $dir already has a www/: not mixing another Bundle into it" >&2 && exit 1; }

if [ "$release" = --local ]; then
  www="$(dirname "$0")/../bootstrap/www"
  [ -f "$www/__tooob/version.json" ] || { echo "bundle.sh: build it first: bootstrap/build.sh" >&2 && exit 1; }
  cp -R "$www" "$dir/www"
  what="bootstrap/www as built here"
else
  if [ -n "$release" ]; then from="$RELEASES/download/v${release#v}"; else from="$RELEASES/latest/download"; fi
  for file in tooob-bootstrap.tar.xz tooob-bootstrap.tar.xz.sha256; do
    curl -fsSL -o "$dir/$file" "$from/$file" || { echo "bundle.sh: couldn't download $from/$file" >&2 && exit 1; }
  done
  (cd "$dir" && shasum -a 256 -c --quiet tooob-bootstrap.tar.xz.sha256) ||
    { echo "bundle.sh: tooob-bootstrap.tar.xz doesn't match its checksum: not using it" >&2 && exit 1; }
  tar -xJf "$dir/tooob-bootstrap.tar.xz" -C "$dir"
  rm "$dir/tooob-bootstrap.tar.xz" "$dir/tooob-bootstrap.tar.xz.sha256"
  mv "$dir/tooob-bootstrap" "$dir/www"
  what="the Bootstrap Bundle of Release $(sed -n 's/^ *"release": "\(.*\)",$/\1/p' "$dir/www/__tooob/version.json")"
fi

echo "bundle.sh: $what, Bundle hash $(shasum -a 256 "$dir/www/__tooob/version.json" | cut -d' ' -f1)"
