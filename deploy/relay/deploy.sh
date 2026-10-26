#!/bin/sh
# Deploy a Relay to a VPS over SSH, or upgrade one: the Relay image (image/) with a Release's
# Bootstrap Bundle, built and run there by Docker Compose. Upgrading a Relay's Bootstrap is running
# this again with a newer Release (#67). wizard.sh runs it the first time.
#   ./deploy.sh [RELEASE | --local] [--dry-run]   (see ../bundle.sh for RELEASE and --local)
# Its settings are in .env here, which wizard.sh writes: VPS_IP, SSH_USER, RELAY_DOMAIN,
# DNS_PROVIDER, DNS_API_TOKEN and ACME_EMAIL.
set -eu
cd "$(dirname "$0")"
[ -f .env ] || { echo "deploy.sh: no .env here: run ./wizard.sh first" >&2 && exit 1; }
. ./.env
DNS_PROVIDER=${DNS_PROVIDER:-cloudflare}
for v in VPS_IP SSH_USER RELAY_DOMAIN DNS_API_TOKEN ACME_EMAIL; do
  eval "[ -n \"\${$v:-}\" ]" || { echo "deploy.sh: .env has no $v: run ./wizard.sh" >&2 && exit 1; }
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
cp compose.yaml "$ctx/"
cp -R image "$ctx/image"
../bundle.sh "$ctx/image" ${release:+"$release"}
(umask 077 && printf "RELAY_DOMAIN='%s'\nDNS_PROVIDER='%s'\nDNS_API_TOKEN='%s'\nACME_EMAIL='%s'\n" \
  "$RELAY_DOMAIN" "$DNS_PROVIDER" "$DNS_API_TOKEN" "$ACME_EMAIL" >"$ctx/.env")

if [ $dry_run = yes ]; then
  echo "deploy.sh: --dry-run, so not deploying to $SSH_USER@$VPS_IP"
  exit 0
fi
vps() { ssh -o StrictHostKeyChecking=accept-new -o ConnectTimeout=10 "$SSH_USER@$VPS_IP" "$@"; }
# A fresh image/ each time, so no file of an older Bundle lingers in the new one.
tar -C "$ctx" -cf - . | vps 'rm -rf tooob-relay/image && mkdir -p tooob-relay && tar -C tooob-relay -xf -'
vps 'cd tooob-relay && docker compose up -d --build --remove-orphans'
../served.sh "$RELAY_DOMAIN" "$ctx/image/www"
