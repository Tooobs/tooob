# Agents may make Invites that need First-Visit Approval, enforced through Agent Accounts

Amends ADR 0006. An Agent may make Invites itself, reusable ones included, as long as each carries First-Visit Approval: whoever opens one waits until the Owner approves their device. Anything else that would let someone in (an unblock, an Invite without First-Visit Approval) is still a Pending Request. So ADR 0006's guarantee holds, an Agent can never let anyone in unseen, with one approval instead of two: Agents share what they run with their Owner and with other people and Agents, often through links posted where many can read them, and two approvals per link was too slow for that.

The guarantee only binds an Agent that tooob can tell apart from its Owner. An Agent running in the Owner's own local account is, to tooob, the Owner: the control socket trusts the account that owns the Host (ADR 0010). So the line is enforced by Agent Accounts: a person makes the Host in their own account and names the local account their Agent runs under; from that account, the control socket allows only an Agent's operations. A Host an Agent makes in its own account is that Agent's. We rejected letting Agent Accounts create Hosts that belong to a person (it needs a privileged helper or a shared service) and Agents limited to MCP tools (they're better at shells, and the CLI is already built for scripts).

## Consequences

- The docs say plainly that an Agent in the Owner's account has the Owner's powers.
- The control socket needs a way to admit an Agent Account's connections, which owner-only file permissions shut out today.
