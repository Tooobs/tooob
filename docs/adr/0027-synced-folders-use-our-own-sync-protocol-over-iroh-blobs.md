# Synced Folders use a sync protocol of our own, with iroh-blobs moving the bytes

The two Hosts of a Synced Folder keep it in step with a small protocol of our own: each tells the other what it holds and what changed, and `iroh-blobs` moves the files' bytes, verified (BLAKE3), streamed from disk and resumed after a cut. A change is ordered by version counters each box keeps, never by clocks, so two changes made apart are seen as a conflict and both kept, and a box with a wrong clock does no harm.

We rejected `iroh-docs` (with `iroh-blobs` under it), n0's multi-writer store. It has no folder watching or renames, so we'd write the sync ourselves anyway. It settles two writes to one key by timestamp and drops the loser silently, where a Synced Folder must keep both. It refuses entries more than 10 minutes ahead of the receiver's clock. And n0 promises it no stability: its 0.98, 0.99 and 0.100 Releases each broke, its storage moved twice, and its open bugs (a sync that wedges, blobs not fetched after a sync, state corrupted by a crash) are the ones folder sync would hit (researched 2026-11-04). We also rejected writing the transfer ourselves on plain iroh streams: verified, resumable transfer of any size is what `iroh-blobs` already does well.

## Consequences

- `iroh-blobs` is 0.x with no stability promise either, so tooob pins its exact version, and the sync protocol's identifier carries a **Sync Protocol Version** (`tooob-sync/0`), as ADR 0017 does for the Bootstrap: two boxes on Releases that can't sync say "update the other box", and never half-sync. Bumping `iroh-blobs` across a wire change is a new Sync Protocol Version.
- The protocol is built for two boxes but numbers versions per box, so three or more can come later without a new model.
