---
status: amended by ADR 0020
---

# tooob ships no default Relay or Bootstrap

Every Host's settings name the Relay its Visitors reach it through (or none: this network only) and the Bootstrap its Invite Links open. `tooob add` and `tooob share` refuse to guess either. A default would point every new Host at servers someone else runs. Those Relay Operators would see which keys talk, when, and from where (ADR 0007), and every Host would depend on their cost and uptime without its Owner having chosen them. We also rejected n0's public Relays as a default: they're throttled, and they're there for iroh's development, not as a service. The price is a slower first run: an Owner stands up a Relay and a Bootstrap first (`deploy/` has both), or uses `--local` to stay on this network.
