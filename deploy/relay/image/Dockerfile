# The Relay image (ADR 0020, #67), which both Relay setups run: stock iroh-relay, and Caddy in front
# of it that gets a wildcard certificate through a DNS challenge, hands iroh's paths to iroh-relay
# and serves the Bootstrap Bundle in www/ for everything else. The deploy scripts build it from a
# copy of this folder, with the Bundle put in www/ by ../../bundle.sh. DNS_PROVIDER names the
# caddy-dns module to build in (github.com/caddy-dns/<provider>): one configured by just a token.
ARG DNS_PROVIDER=cloudflare

FROM caddy:2.10-builder AS caddy
ARG DNS_PROVIDER
RUN xcaddy build --with github.com/caddy-dns/${DNS_PROVIDER}

FROM n0computer/iroh-relay:v1.3.0 AS iroh-relay

FROM caddy:2.10-alpine
ARG DNS_PROVIDER
ENV DNS_PROVIDER=${DNS_PROVIDER}
COPY --from=caddy /usr/bin/caddy /usr/bin/caddy
COPY --from=iroh-relay /iroh-relay /usr/bin/iroh-relay
COPY iroh-relay.toml /etc/iroh-relay.toml
COPY Caddyfile front.caddy /etc/caddy/
COPY --chmod=755 start.sh /usr/bin/start-relay
COPY www /srv
EXPOSE 80 443
CMD ["start-relay"]
