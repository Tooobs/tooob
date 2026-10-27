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

Put the Relay's domain on the [Public Suffix List](https://publicsuffix.org). Each Key Address is already an origin of its own, but on the list each one is also a *site* of its own: one Site can't set cookies that reach the others, and browsers keep each one in its own process. Getting on it means a pull request to the list's private section and a `_psl` TXT record on the domain ([guidelines](https://github.com/publicsuffix/list/wiki/Guidelines)), and it takes weeks. The Relay works in the meantime.

## The Bootstrap a Relay serves

Each deploy downloads a Release's Bootstrap Bundle (`deploy.sh 0.1.0`, or the newest Release by default), checks it against the Release's checksum (`bundle.sh`), and prints its hash to compare with the Release's notes. Upgrading a Relay's Bootstrap means deploying it again with a newer Release. `--local` serves `bootstrap/www` as built here instead.

Afterwards, `served.sh` checks that the Relay's domain and a Key Address under it both serve that Bundle.

## Checks

`cd check && node --test bundle.test.mjs` tests `bundle.sh`. `node --test relay.test.mjs` builds the Relay image and runs it in Docker, and a real Host and headless Chrome open an Invite Link through it. That one needs `bootstrap/build.sh` and `npm install` in `bootstrap/check` first, and leaves out the certificates, which only a real domain can get.
