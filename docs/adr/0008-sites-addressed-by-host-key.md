---
status: amended by ADR 0015 and ADR 0020
---

# Sites are addressed by their Host Key

_Amended by ADR 0015: the domain is the Bootstrap's, whoever runs it; tooob names none._

A Site's address is its Key Address, `<host key>.<bootstrap domain>`; Friendly Names are a later layer on top. Each Host gets its own origin, so browsers isolate Sites from each other, and the Bootstrap's domain goes on the Public Suffix List so subdomains can't share cookies. Addresses can't be forged and need no registry. They're ugly, but Visitors click Invite Links rather than typing them. Hard to reverse: every Invite Link and bookmark carries the domain.
