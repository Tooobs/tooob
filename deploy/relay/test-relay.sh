#!/bin/sh
# A closed Relay on your own network to try things on (#142): the Relay image (image/) run by podman
# over plain http, no domain, no certificate, its Enrolled Keys in a podman volume of its own. Hosts,
# `tooob-relay` and the enroll API work on it as on a real Relay; browser Visits don't (a service
# worker needs https). See README.md, "A test Relay on your own network".
#   ./test-relay.sh start HOST:PORT SIGN_UP_URL [RELEASE | --local]   (see ../bundle.sh)
#   ./test-relay.sh stop
# HOST:PORT is this machine's address on your network, as Hosts will name it. Starting again
# rebuilds it with that Bundle and keeps its Enrolled Keys and Operator Tokens.
set -eu
cd "$(dirname "$0")"
name=tooob-test-relay
usage="usage: ./test-relay.sh start HOST:PORT SIGN_UP_URL [RELEASE | --local] | stop"

case ${1:-} in
  stop)
    podman rm -f "$name" >/dev/null 2>&1 || true
    echo "test-relay.sh: stopped; its Enrolled Keys stay in the podman volume $name-data"
    exit 0
    ;;
  start) [ $# -ge 3 ] || { echo "$usage" >&2 && exit 1; } ;;
  *) echo "$usage" >&2 && exit 1 ;;
esac
address=$2 sign_up=$3 release=${4:-}
port=${address##*:}

ctx=$(mktemp -d)
trap 'rm -rf "$ctx"' EXIT
cp -R image "$ctx/image"
mkdir "$ctx/image/tooob-relay"
cp -R ../../relay/Cargo.toml ../../relay/Cargo.lock ../../relay/src ../../relay/iroh-relay "$ctx/image/tooob-relay/"
../bundle.sh "$ctx/image" ${release:+"$release"}
podman build -t "$name" "$ctx/image"

podman rm -f "$name" >/dev/null 2>&1 || true
# The first start closes it (ADR 0030: its first Enrolled Key closes a Relay) before it ever serves:
# a stand-in key, fresh, whose secret is thrown away at once, so no Host can ever be it. Enrolled
# into a new volume only: remove it later and the Relay stays open across restarts.
if ! podman volume exists "$name-data"; then
  podman volume create "$name-data" >/dev/null
  closer=$(openssl genpkey -algorithm ed25519 | openssl pkey -pubout -outform DER | tail -c 32 | od -An -tx1 | tr -d ' \n')
  podman run --rm -v "$name-data:/data" "$name" \
    tooob-relay enroll "$closer" --label "stand-in: keeps this test Relay closed"
fi
podman run -d --name "$name" \
  -p "$port:8080" \
  -p 127.0.0.1:8081:8081 \
  -v "$name-data:/data" \
  -e RELAY_PLAIN_HTTP=yes \
  -e RELAY_DOMAIN="$address" \
  -e TOOOB_RELAY_SIGN_UP_URL="$sign_up" \
  "$name" >/dev/null
echo "test-relay.sh: http://$address/ is up, its Sign-up Page $sign_up"
podman exec "$name" tooob-relay list
echo "  its admin page is on 127.0.0.1:8081 here (ssh -L 8081:127.0.0.1:8081 to reach it)"
echo "  tooob-relay there: podman exec $name tooob-relay list (or token new, enroll, code)"
