#!/bin/sh
# Deploy a Relay to Fly.io, or upgrade one: the Relay image (../relay/image) with a Release's
# Bootstrap Bundle, built by Fly. Upgrading a Relay's Bootstrap is running this again with a newer
# Release (#67). The first time, README.md first: the app, its IPv4, volume, DNS token and DNS records.
#   ./deploy.sh [RELEASE | --local] [--dry-run]   (see ../bundle.sh for RELEASE and --local)
# Its settings come from the environment: FLY_APP, RELAY_DOMAIN, ACME_EMAIL, and DNS_PROVIDER
# (default cloudflare). The DNS API token is the app's secret DNS_API_TOKEN.
set -eu
cd "$(dirname "$0")"
DNS_PROVIDER=${DNS_PROVIDER:-cloudflare}
for v in FLY_APP RELAY_DOMAIN ACME_EMAIL; do
  eval "[ -n \"\${$v:-}\" ]" || { echo "deploy.sh: set $v (see README.md)" >&2 && exit 1; }
done

dry_run=no release=""
for arg in "$@"; do
  case $arg in
    --dry-run) dry_run=yes ;;
    --local) release=--local ;;
    -*) echo "usage: ./deploy.sh [RELEASE | --local] [--dry-run]" >&2 && exit 1 ;;
    *) release=$arg ;;
  esac
done

ctx=$(mktemp -d)
trap 'rm -rf "$ctx"' EXIT
cp -R ../relay/image/. "$ctx/"
cp fly.toml "$ctx/"
../bundle.sh "$ctx" ${release:+"$release"}

if [ $dry_run = yes ]; then
  echo "deploy.sh: --dry-run, so not deploying to $FLY_APP"
  exit 0
fi
(cd "$ctx" && fly deploy -a "$FLY_APP" --ha=false --yes --build-arg "DNS_PROVIDER=$DNS_PROVIDER" \
  --env "RELAY_DOMAIN=$RELAY_DOMAIN" --env "ACME_EMAIL=$ACME_EMAIL")
../served.sh "$RELAY_DOMAIN" "$ctx/www"
