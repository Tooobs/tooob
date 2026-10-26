#!/bin/sh
# Wait until a Relay serves the Bootstrap Bundle in BUNDLE (a folder), on its domain and on a Key
# Address under it (through its wildcard certificate), then say so: the end of each Relay setup's
# deploy.sh. The first deploy waits longest, while Caddy gets its certificates.
#   ./served.sh RELAY_DOMAIN BUNDLE
set -eu
domain=$1 bundle=$2
want=$(shasum -a 256 "$bundle/__tooob/version.json" | cut -d' ' -f1)
# check.<domain>: any name under the wildcard is a Key Address as far as the Relay is concerned.
for url in "https://$domain/__tooob/version.json" "https://check.$domain/__tooob/version.json"; do
  printf 'served.sh: waiting for %s ' "$url"
  tries=0
  until [ "$(curl -fsS -m 10 "$url" 2>/dev/null | shasum -a 256 | cut -d' ' -f1)" = "$want" ]; do
    tries=$((tries + 1))
    [ $tries -lt 60 ] || { printf '\nserved.sh: %s is still not serving this Bundle after 5 minutes\n' "$url" >&2 && exit 1; }
    printf '.'
    sleep 5
  done
  printf ' ok\n'
done
echo "served.sh: https://$domain/ serves the Bundle, hash $want"
