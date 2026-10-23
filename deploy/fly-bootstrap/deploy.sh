#!/bin/sh
# Assemble a build context (Dockerfile + bootstrap/www with its built WASM) and deploy it.
# Build the Bootstrap first: bootstrap/build.sh.
set -e
cd "$(dirname "$0")"
www=../../bootstrap/www
[ -f "$www/__tooob/pkg/web_bg.wasm" ] || { echo "build it first: bootstrap/build.sh"; exit 1; }
ctx=$(mktemp -d)
cp Dockerfile Caddyfile fly.toml "$ctx/"
cp -R "$www" "$ctx/www"
(cd "$ctx" && fly deploy --ha=false --yes)
rm -rf "$ctx"
