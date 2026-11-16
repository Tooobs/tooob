#!/bin/sh
# Whether a Relay serves a Bootstrap Bundle on its domain and on a Key Address under it (through
# its wildcard certificate).
#   ./served.sh RELAY_DOMAIN BUNDLE: wait until both serve the Bundle in BUNDLE (a folder), then
#     say so: the end of each Relay setup's deploy.sh. The first deploy waits longest, while Caddy
#     gets its certificates.
#   ./served.sh RELAY_DOMAIN: check that both serve the same Bundle, the one the domain serves
#     now, and name it: a live check after a deploy, needing no Bundle at hand.
# SERVED_TRIES and SERVED_EVERY (seconds) set how long it waits: 60 tries 5 s apart by default.
set -eu
[ $# -ge 1 ] && [ $# -le 2 ] || { echo "usage: ./served.sh RELAY_DOMAIN [BUNDLE]" >&2 && exit 2; }
domain=$1
tries_allowed=${SERVED_TRIES:-60} every=${SERVED_EVERY:-5}
hash_of() { shasum -a 256 | cut -d' ' -f1; }
if [ $# -eq 2 ]; then
  want=$(hash_of < "$2/__tooob/version.json")
else
  # Byte for byte, as the loop below hashes it: through a file, since $(…) drops trailing newlines.
  answer=$(mktemp)
  trap 'rm -f "$answer"' EXIT
  curl -fsS -m 10 -o "$answer" "https://$domain/__tooob/version.json" 2>/dev/null ||
    { echo "served.sh: https://$domain/__tooob/version.json serves no Bundle" >&2 && exit 1; }
  want=$(hash_of < "$answer")
fi
# check.<domain>: any name under the wildcard is a Key Address as far as the Relay is concerned.
for url in "https://$domain/__tooob/version.json" "https://check.$domain/__tooob/version.json"; do
  printf 'served.sh: waiting for %s ' "$url"
  tries=0
  until [ "$(curl -fsS -m 10 "$url" 2>/dev/null | hash_of)" = "$want" ]; do
    tries=$((tries + 1))
    [ $tries -lt "$tries_allowed" ] || { printf '\nserved.sh: %s is still not serving this Bundle after %s tries\n' "$url" "$tries_allowed" >&2 && exit 1; }
    printf '.'
    sleep "$every"
  done
  printf ' ok\n'
done
echo "served.sh: https://$domain/ serves the Bundle, hash $want"
