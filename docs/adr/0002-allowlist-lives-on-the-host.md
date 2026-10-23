# The Allowlist lives only on the Host; Relays can only deny

Every decision about who may reach a Site is made on the Host. Relays and Bootstraps never hold the Allowlist or learn who an Owner's Visitors are. A Host may ask its Relay for an Upstream Deny (flood protection), signed with the Host Key, but nothing upstream can ever admit anyone. We rejected a hosted control plane holding access lists: it would be more convenient (one dashboard, blocking at the edge), but a breached or malicious control plane could then add Visitors to anyone's home.

## Consequences

- The Host always enforces a Block itself; the Relay never sees traffic on the Direct Path, so an Upstream Deny is only a supplement.
- The management UI is served by the Host (as an owner-only Site), not by the Bootstrap.
