# The Relay image (ADR 0020, #67), which both Relay setups run: tooob-relay (iroh-relay with Enrolled
# Keys, #128), and Caddy in front of it that gets a wildcard certificate through a DNS challenge,
# hands iroh's paths and /__tooob/relay.json to tooob-relay
# and serves the Bootstrap Bundle in www/ for everything else. The deploy scripts build it from a
# copy of this folder, with the Bundle put in www/ by ../../bundle.sh. DNS_PROVIDER names the
# caddy-dns module to build in (github.com/caddy-dns/<provider>): one configured by just a token.
ARG DNS_PROVIDER=cloudflare

# Caddy 2.11 or later: 2.10.2 answers every precompressed file with a 206 (caddyserver/caddy#7250),
# which a service worker's cache refuses.
FROM docker.io/library/caddy:2.11-builder AS caddy
ARG DNS_PROVIDER
RUN xcaddy build --with github.com/caddy-dns/${DNS_PROVIDER}

# tooob-relay (#128, ADR 0030): iroh-relay 1.3.0 forwarding only traffic to or from an Enrolled Key,
# built from its source, which the deploy scripts put in tooob-relay/. Alpine's Rust links musl, so
# the binary runs in the Caddy image below.
FROM docker.io/library/rust:1.91-alpine3.22 AS tooob-relay
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY tooob-relay .
RUN cargo build --release --locked && cp target/release/tooob-relay /tooob-relay

# The Bundle with a brotli and a gzip copy beside each file a browser is sent compressed (#113),
# made here so the Bundle itself doesn't change, and in a stage of its own so brotli stays out of
# the Relay's image.
FROM docker.io/library/alpine:3.22 AS bundle
RUN apk add --no-cache brotli
COPY --chmod=755 precompress.sh /usr/bin/precompress
COPY www /srv
RUN precompress /srv

FROM docker.io/library/caddy:2.11-alpine
ARG DNS_PROVIDER
ENV DNS_PROVIDER=${DNS_PROVIDER}
COPY --from=caddy /usr/bin/caddy /usr/bin/caddy
COPY --from=tooob-relay /tooob-relay /usr/bin/tooob-relay
COPY iroh-relay.toml /etc/iroh-relay.toml
COPY Caddyfile plain-http.Caddyfile front.caddy /etc/caddy/
# A config Caddy rejects fails the build, before it can replace a running Relay: parsed (adapted)
# with stand-ins for what the Relay's environment sets. Not `caddy validate`: that also provisions
# the DNS provider, which refuses a stand-in API token.
RUN RELAY_DOMAIN=relay.example.com DNS_API_TOKEN=stand-in ACME_EMAIL=relay@example.com \
    caddy adapt --config /etc/caddy/Caddyfile --adapter caddyfile >/dev/null && \
    caddy adapt --config /etc/caddy/plain-http.Caddyfile --adapter caddyfile >/dev/null
COPY --chmod=755 start.sh /usr/bin/start-relay
COPY --from=bundle /srv /srv
# 8080 only with RELAY_PLAIN_HTTP set (plain-http.Caddyfile, a test Relay).
EXPOSE 80 443 8080
CMD ["start-relay"]
