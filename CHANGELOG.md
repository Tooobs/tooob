# Changelog

Each Release of tooob: the `tooob` command, the Bootstrap Bundle and the Host's container image,
all with one Release number (ADR 0017). Release numbers follow [SemVer](https://semver.org); while
they're `0.x`, expect the commands to change. Add to "Unreleased" as part of each change;
`./release.sh` turns it into the next Release's section.

## Unreleased

The first Release, for technical early adopters on Linux and macOS.

### Added
- **A Host** makes one Site reachable to invited Visitors in ordinary browsers, over iroh, through
  a Relay that can't read the traffic, and straight over WebRTC when it can: `tooob add`, `run`,
  `share`, `invite`, `pair`, `visitors`, `invites`, `block`, `approve`, `unblock`, `revoke`,
  `stop`, `ls`, `show` and `rm`.
- **The Bootstrap Bundle**, what a Host's Invite Links open, with a manifest
  (`__tooob/version.json`) naming its Release, its Protocol Version and every file's hash.
- **Key Addresses**: each Host is an origin of its own, `<host key>.<relay domain>`, and its Invite
  Links are `https://<host key>.<relay domain>/#invite=<token>`, so no Site's scripts can reach
  another's browser storage or Visitor Key. A Host's settings name only its Relay, which serves
  the Bootstrap (ADR 0020); Host Keys are written in z-base-32, short enough to be a hostname's label.
- **Relay setups** (`deploy/`): one container, stock iroh-relay behind Caddy, which gets a
  wildcard certificate through a DNS challenge and serves a Release's Bootstrap Bundle, checked
  against its checksum, on the Relay's domain and every Key Address. Run it on a VPS
  (`deploy/relay/`, with a wizard) or on Fly.io (`deploy/fly-relay/`). `deploy/fly-bootstrap/` is
  gone: a Relay is all a Host needs.
- **A Relay for its Owner alone needs no Public Suffix List** (ADR 0021). A cookie a Site sets for
  the whole Relay domain stays with that Site, and the shell deletes any it finds. A Relay passes
  no `Cookie` header on, and asks browsers for a process per Key Address (`Origin-Agent-Cluster`,
  `Cross-Origin-Opener-Policy`). A Relay shared by people who don't trust each other still goes on
  the list, for password managers: `deploy/README.md` says why.
- **A Site's pages have their own addresses** (ADR 0023): `https://<host key>.<relay domain>/<path>`
  opens that page, and the address bar and the tab's title follow the Site, so a reload, a bookmark
  or a shared link lands where it was. A Relay serves the shell at any path that isn't iroh's or a
  Bundle file; once installed, the service worker serves it for every page load, fetched as `/`.
- **`tooob --version`** prints the Release and the Protocol Version; a starting Host logs them.
