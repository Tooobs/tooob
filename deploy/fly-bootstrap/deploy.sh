#!/bin/sh
# Deploy a Bootstrap to Fly.io: a Release's Bootstrap Bundle, checked against its checksum, or
# bootstrap/www as built here (#59).
#   ./deploy.sh [RELEASE]   that Release's Bundle, e.g. 0.1.0 (default: the newest Release)
#   ./deploy.sh --local     bootstrap/www, built first with bootstrap/build.sh
#   --dry-run               do everything but `fly deploy`
# TOOOB_RELEASES points at another repo's Releases (a fork's), as GitHub lays them out.
set -eu
cd "$(dirname "$0")"
RELEASES=${TOOOB_RELEASES:-https://github.com/Tooobs/tooob/releases}

dry_run=no release="" local=no
for arg in "$@"; do
  case $arg in
    --dry-run) dry_run=yes ;;
    --local) local=yes ;;
    -*) echo "usage: ./deploy.sh [RELEASE | --local] [--dry-run]" >&2 && exit 1 ;;
    *) release=$arg ;;
  esac
done

ctx=$(mktemp -d)
trap 'rm -rf "$ctx"' EXIT
cp Dockerfile Caddyfile fly.toml "$ctx/"

if [ $local = yes ]; then
  www=../../bootstrap/www
  [ -f "$www/__tooob/version.json" ] || { echo "deploy.sh: build it first: bootstrap/build.sh" >&2 && exit 1; }
  cp -R "$www" "$ctx/www"
  what="bootstrap/www as built here"
else
  if [ -n "$release" ]; then from="$RELEASES/download/v${release#v}"; else from="$RELEASES/latest/download"; fi
  for file in tooob-bootstrap.tar.xz tooob-bootstrap.tar.xz.sha256; do
    curl -fsSL -o "$ctx/$file" "$from/$file" || { echo "deploy.sh: couldn't download $from/$file" >&2 && exit 1; }
  done
  (cd "$ctx" && shasum -a 256 -c --quiet tooob-bootstrap.tar.xz.sha256) ||
    { echo "deploy.sh: tooob-bootstrap.tar.xz doesn't match its checksum: not deploying it" >&2 && exit 1; }
  tar -xJf "$ctx/tooob-bootstrap.tar.xz" -C "$ctx"
  mv "$ctx/tooob-bootstrap" "$ctx/www"
  what="the Bootstrap Bundle of Release $(sed -n 's/^ *"release": "\(.*\)",$/\1/p' "$ctx/www/__tooob/version.json")"
fi

# The Bundle's hash, to compare with its Release's notes.
echo "deploy.sh: $what, Bundle hash $(shasum -a 256 "$ctx/www/__tooob/version.json" | cut -d' ' -f1)"
if [ $dry_run = yes ]; then
  echo "deploy.sh: --dry-run, so not deploying"
else
  (cd "$ctx" && fly deploy --ha=false --yes)
fi
