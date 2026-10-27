# A Relay on Fly.io

The Relay image (`../relay/image`, see [../README.md](../README.md)) as a Fly app. Caddy terminates TLS itself, so Fly passes TCP straight through. That takes a dedicated IPv4, and Caddy's certificates live on a volume.

## The first time

With `fly` installed and logged in, and `FLY_APP` set to a name of your own:

```sh
fly apps create "$FLY_APP"
fly ips allocate-v4 -a "$FLY_APP"                 # dedicated: raw TCP can't use a shared IPv4
fly ips allocate-v6 -a "$FLY_APP"
fly volumes create caddy_data --size 1 --region sjc -a "$FLY_APP" --yes
fly secrets set DNS_API_TOKEN='…' -a "$FLY_APP" --stage
fly ips list -a "$FLY_APP"                        # the addresses for DNS
```

Then, at your DNS provider, add A and AAAA records pointing at those addresses for the Relay's domain and its wildcard (`relay.example.com` and `*.relay.example.com`; "DNS only" on Cloudflare). A resolver that looked up a name under the wildcard just before the record existed can keep saying it doesn't exist for a few minutes; your DNS provider's own nameservers have it at once (`dig check.relay.example.com @<one of them>`). The token is one that can edit the zone's records: on Cloudflare, My Profile → API Tokens → "Edit zone DNS", limited to that zone.

**Turning an existing Fly app into a Relay**, such as one that ran stock iroh-relay: skip `fly apps create`, and before the first deploy destroy its machines (`fly machines list`, then `fly machine destroy <id> --force`). Fly can't attach the volume to a machine made without one. The app's `<app>.fly.dev` address stops working: the Relay only has certificates for its own domain.

## Deploy

```sh
FLY_APP=… RELAY_DOMAIN=relay.example.com ACME_EMAIL=you@example.com ./deploy.sh [RELEASE]
```

It builds the Relay image with that Release's Bootstrap Bundle (`DNS_PROVIDER` picks the caddy-dns module; the default is `cloudflare`), deploys it, then waits until `https://relay.example.com/` and a Key Address under it serve that Bundle. Upgrading the Relay's Bootstrap is running it again with a newer Release.

Point Hosts at it with `tooob add … --relay https://relay.example.com/`. As shipped, anyone can use it: before you share it, set `access` and `limits` in `../relay/image/iroh-relay.toml`.
