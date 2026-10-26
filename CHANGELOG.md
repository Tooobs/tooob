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
- **`tooob --version`** prints the Release and the Protocol Version; a starting Host logs them.
