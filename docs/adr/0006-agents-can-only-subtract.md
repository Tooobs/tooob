---
status: amended by ADR 0018
---

# Agents can remove access but only propose granting it

An Agent may read, Block and revoke directly. Anything that would let someone in (an Invite, an unblock, an Allowlist change) becomes a Pending Request that the Owner approves outside the Agent's conversation. Agents read content Visitors can influence (Labels, logs, Site pages), so prompt injection is expected; this way an injected or confused Agent can lock people out, which is annoying and reversible, but can never let anyone in. Invites an Agent proposes also carry First-Visit Approval once the Owner approves them, so a leaked Agent-made link still can't let a stranger in unseen.
