#!/bin/sh
# tooob-relay and Caddy, side by side. When either stops on its own, so does the other, and the
# container exits as failed for its restart policy (Compose's or Fly's) to start both afresh.
# Its Enrolled Keys are on the /data volume (/data/tooob-relay/enrolled.toml): none, and it's open.
# Its admin page (#133) logs in with an Operator Token, on port 8081, which no front route and no
# public port reaches: a tunnel is the way in (fly proxy, or ssh -L to a VPS's loopback).
tooob-relay --config-path /etc/iroh-relay.toml --admin-addr '[::]:8081' --status-addr 127.0.0.1:3341 &
relay=$!
# Plain http on port 8080 only when RELAY_PLAIN_HTTP is set (#142, a test Relay on your own
# network); otherwise https on the Relay's domain, as every deploy runs it.
caddyfile=/etc/caddy/Caddyfile
[ -z "${RELAY_PLAIN_HTTP:-}" ] || caddyfile=/etc/caddy/plain-http.Caddyfile
caddy run --config "$caddyfile" &
caddy=$!
asked=""
trap 'asked=yes; kill $relay $caddy 2>/dev/null' INT TERM
while kill -0 $relay 2>/dev/null && kill -0 $caddy 2>/dev/null; do sleep 1; done
kill $relay $caddy 2>/dev/null
wait
[ -n "$asked" ]
