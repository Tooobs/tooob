# Running a Relay

tooob names no Relay of its own ([ADR 0015](../docs/adr/0015-no-default-relay-or-bootstrap.md)): every Host's settings name one, so you run it, or use one run by someone you trust. A Relay also serves the Bootstrap, the page a Host's Invite Links open ([ADR 0020](../docs/adr/0020-a-relay-serves-its-bootstrap.md)), so a Relay is all a Host needs.

Both setups here run the same container (`relay/image/`): stock [iroh-relay](https://github.com/n0-computer/iroh) 1.3.0 behind [Caddy](https://caddyserver.com). Caddy hands iroh's paths (`/relay`, `/ping`, `/generate_204`, `/healthz`) to iroh-relay. Everything else gets a Release's Bootstrap Bundle, on the Relay's domain and on every Key Address under it (`<host key>.<relay domain>`).

- **`relay/`**: a VPS with Docker. `wizard.sh` walks through it all, Hetzner, DNS, firewall, the first deploy and enrolling your first Host, and `deploy.sh` upgrades it.
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

## Closing an open Relay

A Relay with no Enrolled Keys is open: it carries anyone's traffic ([ADR 0030](../docs/adr/0030-a-relay-carries-only-traffic-that-touches-an-enrolled-key.md)). A new Relay closes in its first-time steps (the VPS wizard's "Close the Relay" stage, the Fly README's "Close it"). A Relay deployed before then is still open, and a redeploy never closes it: `deploy.sh` never enrolls anything. Until its first enrollment, `https://relay.example.com/__tooob/relay.json` says `"relay_access":"open"`, and after it `"enrolled"`.

The first enrollment closes it at once (the Relay rereads its Enrolled Keys every quarter second). From then on a Host whose key isn't enrolled loses the Relayed Path, and its Visitors can't reach it through this Relay, until its key is enrolled too. So close it without cutting anyone off:

1. **Collect every Host Key first.** Each Owner of a Host on the Relay runs `tooob show NAME` and sends you the Host Key it prints. A Host Key isn't secret.
2. **Enroll them all at once**, one `tooob-relay enroll <host key> --label <name>` each, back to back, inside the Relay's container. On a VPS: `ssh root@<server>`, then `cd tooob-relay && docker compose exec relay sh`. On Fly.io: `fly ssh console -a <app>`, which opens a shell in the container. Then `tooob-relay list` to check.
3. **Owners of those Hosts do nothing.** The Relay forwards by its file of Enrolled Keys, so a running Host carries on without a break. Its own status rechecks the Relay every 10 minutes, so it can say "not carried" for up to that long.
4. **A Host you missed** gets its key enrolled the same way, or its Owner gets an Enrollment Code (`tooob-relay code --label <name>`) and runs `tooob enroll NAME <code>`. Until then it's cut off from this Relay.

Don't hand out Enrollment Codes to close an open Relay: the first one redeemed closes it while the other Hosts still wait to redeem theirs. Codes are for Hosts added once it's closed.

## A Sign-up Page

A Relay with Enrolled Keys carries only their traffic ([ADR 0030](../docs/adr/0030-a-relay-carries-only-traffic-that-touches-an-enrolled-key.md)). To let Owners enroll themselves, after a payment, a membership or an invite list, run a Sign-up Page of your own: tooob ships none ([ADR 0031](../docs/adr/0031-a-relays-sign-up-is-its-operators-own-service-enrolling-through-a-token-api.md)). It checks whatever you like, then enrolls the Host Key through the Relay's enroll API.

- **Name it:** set `TOOOB_RELAY_SIGN_UP_URL=https://join.example.com/` on the Relay (or `tooob-relay --sign-up-url`). `https://relay.example.com/__tooob/relay.json` then says `"sign_up": "https://join.example.com/"`.
- **Make an Operator Token** for the service, inside the Relay's container: `tooob-relay token new sign-up`. It prints the token once; the Enrolled Keys' file keeps only its hash. Make one for each service. `tooob-relay token list` shows their labels, never the tokens, and `tooob-relay token revoke sign-up` makes one fail at once.
- **Enroll a key**, or relabel it if it's enrolled already:

  ```sh
  curl -X PUT https://relay.example.com/__tooob/api/enrolled/<host key> \
    -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
    -d '{"label": "sam@example.com"}'
  ```

- **Remove a key**, which disconnects it at once:

  ```sh
  curl -X DELETE https://relay.example.com/__tooob/api/enrolled/<host key> \
    -H "Authorization: Bearer $TOKEN"
  ```

Each answers 200 and `{"enrolled":…,"relay_access":…}`; 401 with no valid token (nothing changes), 400 for what isn't a key or a body without a `label` (at most 4 KiB), 404 for removing a key that isn't enrolled. After 10 failed tokens from one address, each within a minute of the last, that address gets 429 until a minute has passed. Anyone may still read `GET /__tooob/enrolled/<host key>`.

**Don't remove a key another account still pays for.** A key has one label, and enrolling it again relabels it, so if two of your accounts can enroll the same key, your service tracks that itself and removes it only when none of them still has it.

## The admin page

A Relay has an admin page: its Enrolled Keys with their labels, whether each is connected now or when it was last seen (since the Relay last started), whether the Relay is open or closed, and forms to add, relabel and remove a key (a removed key is disconnected at once) and to make, list and cancel Enrollment Codes. It edits the same file as the `tooob-relay` commands.

**It logs in with an Operator Token** ([ADR 0031](../docs/adr/0031-a-relays-sign-up-is-its-operators-own-service-enrolling-through-a-token-api.md)), the same kind the enroll API takes. Make one inside the Relay's container, such as `tooob-relay token new admin`, and paste it into the page's login form. With no Operator Token made, nobody can log in. Failed logins are counted apart from the enroll API's, so public guesses there never lock you out here: after 10 from one address, each within a minute of the last, that address is refused until a minute has passed. `tooob-relay token revoke admin` ends that token's sessions at once; a session also ends after 12 hours, or when the Relay restarts.

**It is never on the Relay's public address.** tooob-relay serves it on port 8081 inside the container. Neither the front (Caddy) nor any public port reaches it. Reach it through a tunnel, then open <http://localhost:8081/>:

- **Fly.io**: `fly proxy 8081 -a <app>`. The app's `fly.toml` has no `[[services]]` entry for 8081, so it isn't public.
- **A VPS**: `ssh -L 8081:127.0.0.1:8081 <vps>`. Compose publishes the port on the VPS's own loopback only (`127.0.0.1:8081`), never on its public address.

The page refuses requests that don't name `localhost`, and forms posted from any other website. Its session cookie is `HttpOnly` and `SameSite=Strict`, but not `Secure`: the tunnel ends in plain HTTP on localhost, and Safari never sends a `Secure` cookie back over plain HTTP, even there (Chrome and Firefox do). Cookies ignore ports, so while you are logged in, any other server you open on `localhost` in the same browser gets the cookie too: log out, or use a separate browser profile, when you open local servers you don't trust. A code it makes is shown once: copy it then.

## Checks

`cd check && node --test bundle.test.mjs` tests `bundle.sh`. The Relay image itself is checked live, on a real domain with its certificates: deploy it, run `served.sh`, and open an Invite Link through it.
