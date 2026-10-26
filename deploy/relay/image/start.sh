#!/bin/sh
# iroh-relay and Caddy, side by side. When either stops on its own, so does the other, and the
# container exits as failed for its restart policy (Compose's or Fly's) to start both afresh.
iroh-relay --config-path /etc/iroh-relay.toml &
relay=$!
caddy run --config /etc/caddy/Caddyfile &
caddy=$!
asked=""
trap 'asked=yes; kill $relay $caddy 2>/dev/null' INT TERM
while kill -0 $relay 2>/dev/null && kill -0 $caddy 2>/dev/null; do sleep 1; done
kill $relay $caddy 2>/dev/null
wait
[ -n "$asked" ]
