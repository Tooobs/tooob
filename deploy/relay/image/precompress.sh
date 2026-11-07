#!/bin/sh
# Brotli (-q 11) and gzip (-9) copies beside each file of the Bundle in DIR that a browser is sent
# compressed, for front.caddy's `file_server { precompressed br gzip }` (#113). Run by the image's
# Dockerfile, so the Bundle, its version.json and a Release's hashes don't change: the originals
# stay, and a browser decodes a copy back to them. A copy that can't be made fails the build.
#   ./precompress.sh DIR
set -eu
dir=${1:?usage: precompress.sh DIR}
find "$dir" -type f \( -name '*.wasm' -o -name '*.js' -o -name '*.html' -o -name '*.css' \
  -o -name '*.json' \) -exec sh -euc '
  for f; do
    brotli -f -q 11 -o "$f.br" "$f"
    gzip -9 -n -c "$f" >"$f.gz"
  done' sh {} +
