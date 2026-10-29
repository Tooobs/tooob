# Running a Relay

tooob names no Relay of its own ([ADR 0015](../docs/adr/0015-no-default-relay-or-bootstrap.md)): every Host's settings name one, so you run it, or use one run by someone you trust. A Relay also serves the Bootstrap, the page a Host's Invite Links open ([ADR 0020](../docs/adr/0020-a-relay-serves-its-bootstrap.md)), so a Relay is all a Host needs.

Both setups here run the same container (`relay/image/`): stock [iroh-relay](https://github.com/n0-computer/iroh) 1.3.0 behind [Caddy](https://caddyserver.com). Caddy hands iroh's paths (`/relay`, `/ping`, `/generate_204`, `/healthz`) to iroh-relay. Everything else gets a Release's Bootstrap Bundle, on the Relay's domain and on every Key Address under it (`<host key>.<relay domain>`).

- **`relay/`**: a VPS with Docker. `wizard.sh` walks through it all, Hetzner, DNS, firewall and the first deploy, and `deploy.sh` upgrades it.
- **`fly-relay/`**: Fly.io. See its README.

## What a Relay needs

- **A domain of its own**, e.g. `relay.example.com`, with two DNS records pointing at the Relay: the domain itself and `*.relay.example.com`. Every Host's Key Address is a name under it.
- **A DNS API token**: browsers only run the Bootstrap's service worker on an HTTPS page, so every Key Address needs a certificate. The Relay gets a wildcard one from Let's Encrypt through a DNS challenge, which adds a record through your DNS provider's API. Any provider with a [caddy-dns](https://github.com/caddy-dns) module that takes just a token works; the default is Cloudflare.
- **TCP 443** (and 80, which redirects to it). UDP 7842 is optional and these setups leave it off: it's iroh-relay's QUIC address discovery, which helps native endpoints punch holes, and v1 doesn't use it.
- **Generous bandwidth**: every byte a Visitor loads over the Relayed Path passes through it. A Relay stores nothing.
- **A long-running process**: Relay connections are WebSockets that stay open, so serverless platforms that stop between requests won't do.

## The Public Suffix List

A Relay for your own Hosts doesn't need to be on the [Public Suffix List](https://publicsuffix.org) ([ADR 0021](../docs/adr/0021-a-relay-is-safe-for-its-owner-without-the-public-suffix-list.md)). Each Key Address is an origin of its own, so its storage, service worker and Visitor Key are walled off from every other Host's. On the list, each would also be a *site* of its own. Without it, tooob closes what it can of the difference: a cookie a Site sets for the whole Relay domain is kept to that Site, or deleted when a shell next starts. The Relay passes no `Cookie` header on, so a Site can't fill the domain with cookies until other Hosts' Visitors can't connect. And the Relay asks browsers to give each Key Address a process of its own.

**A Relay shared by people who don't trust each other must be on the list before they use it.** Password managers match saved logins by base domain, not origin. Without the list, one Host's page can be offered the logins Visitors saved on another Host's, and only the list stops that. Getting on it means a pull request to the list's private section and a `_psl` TXT record on the domain ([guidelines](https://github.com/publicsuffix/list/wiki/Guidelines)). Volunteers review it, which takes weeks and may end in a no, and browsers pick up the change months later.

## The Bootstrap a Relay serves

Each deploy downloads a Release's Bootstrap Bundle (`deploy.sh 0.1.0`, or the newest Release by default), checks it against the Release's checksum (`bundle.sh`), and prints its hash to compare with the Release's notes. Upgrading a Relay's Bootstrap means deploying it again with a newer Release. `--local` serves `bootstrap/www` as built here instead.

Afterwards, `served.sh` checks that the Relay's domain and a Key Address under it both serve that Bundle.

## Checks

`cd check && node --test bundle.test.mjs` tests `bundle.sh`. The Relay image itself is checked live, on a real domain with its certificates: deploy it, run `served.sh`, and open an Invite Link through it.
