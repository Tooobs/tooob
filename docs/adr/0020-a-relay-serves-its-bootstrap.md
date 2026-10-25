# A Relay serves its Bootstrap, and each Host's Key Address is on the Relay's domain

Amends ADR 0015 and ADR 0008. A Relay Operator serves the Bootstrap beside their Relay, from the Relay's own domain: stock iroh-relay behind one web server that hands iroh's paths to it and serves a Release's Bootstrap Bundle for everything else. So a Host's settings name only its Relay, and a Host's Key Address is `<host key>.<relay domain>`: its Invite Links are `https://<host key>.<relay domain>/#invite=<token>`. Before, every Host also named a Bootstrap, which meant an Owner deploying a static site with wildcard subdomains over HTTPS, something no ordinary Owner can do, and which left Owners choosing two servers and trusting two operators. A Relay's operator already sees its Visitors' metadata, so serving the code their browsers run adds no new party to trust. We rejected a Relay advertising a Bootstrap hosted elsewhere (two servers, two operators, more to go wrong) and keeping them separate with better tooling (still two choices). Still no default (ADR 0015): Owners choose a Relay, and later find one through the Relay finder.

Each Key Address is its own origin, as ADR 0008 decided but v1 hadn't built: Invite Links put the Host Key after the `#`, so every Site on one Bootstrap shared its origin and could read the others' browser storage and use their Visitor Keys. That's tolerable for one Owner's own Bootstrap, and not for Relays many Owners share, so Key Addresses come before shared Relays.

## Consequences

- A Relay Operator needs a wildcard DNS record and certificate for `*.<relay domain>` (a DNS challenge), and puts the domain on the Public Suffix List. `deploy/` sets up both.
- `tooob` checks its Relay's Bootstrap at every start (refusing a different Protocol Version, or none) and now and then while running (a warning).
- A Key Address lasts as long as its Relay: switching Relay is a new origin, so every Visitor needs a new Invite. Like Signal's notice that a contact's safety number changed, `tooob` warns the Owner before a Host's Relay changes, and a returning Visitor is told the Site moved wherever the old Relay can still say so.
- `--bootstrap` goes; `--local` (no Relay) stays only for tooob's own native client in tests, since browsers reach Hosts only through a Relay.
