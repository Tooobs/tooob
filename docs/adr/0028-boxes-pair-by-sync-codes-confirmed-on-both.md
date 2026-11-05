# Two boxes pair for a Synced Folder by Sync Codes the Owner confirms on both

A box joins a Synced Folder with a single-use Sync Invite, made on the box already in it and expiring in 10 minutes. Using it isn't enough: both boxes then show a six-digit Sync Code, worked out from both Host Keys and the invite, and nothing syncs until the Owner has confirmed on each box that the two match. A mismatch, a "no" or a timeout ends the pairing and spends the invite. Once paired, each box syncs only with the other's Host Key, until either leaves.

A paired box gets the Owner's files and can change or delete them, so pairing has to be at least as hard to slip past as an Owner Invite. A code derived from both keys means a box that got hold of the invite and connected in between shows codes that don't match. Confirming on both boxes, not just the inviting one, means neither box trusts a peer the Owner didn't see. We rejected trusting the invite alone (a leaked invite would pair a stranger's box with full write access) and confirming on one box only (the joining box could be talked into syncing with an impostor that holds the invite).

## Consequences

- Pairing counts as letting someone in (ADR 0006, as amended by ADR 0018): an Agent may make a Sync Invite and may leave, but can't confirm a Sync Code, and an Agent Account's control connection is refused one.
- The Sync Code is shown only while pairing, as the Device Code is only while a device waits.
