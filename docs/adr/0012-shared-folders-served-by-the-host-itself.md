# Shared Folders are served by the Host itself, not a bundled web server

A Shared Folder is served from inside the `tooob` process, answering each Visitor's request straight from disk, with no web server alongside and no local port. We rejected bundling nginx (or Caddy). It would mean a second binary to build and patch for every platform, and nginx's Windows build is a beta. It would need a local port, so other accounts on the box could read the folder without an Invite. And it would give two processes to keep in step. It adds no security either, because every access decision is the Host's (ADR 0002). Richer file apps (uploads, thumbnails, search) stay ordinary Sites that the Host forwards to.

## Consequences

- Range requests, streaming, symlink and `..` refusal are ours to get right and to test.
- Nothing in the file-serving path is platform-specific, so a Windows Host only needs the control channel, private files and stop signals ported.
