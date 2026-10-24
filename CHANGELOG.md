# Changelog

Each Release of tooob: the `tooob` command, the Bootstrap Bundle and the Host's container image,
all at one version (ADR 0017). Versions follow [SemVer](https://semver.org); while they're `0.x`,
expect the commands to change. Add to "Unreleased" as part of each change; `./release.sh` turns it
into the next Release's section.

## Unreleased

The first Release, for technical early adopters on Linux and macOS.

### Added
- **A Host** makes one Site reachable to invited Visitors in ordinary browsers, over iroh, through
  a Relay that can't read the traffic, and straight over WebRTC when it can: `tooob add`, `run`,
  `share`, `invite`, `pair`, `visitors`, `invites`, `block`, `approve`, `unblock`, `revoke`,
  `stop`, `ls`, `show` and `rm`.
- **The Bootstrap Bundle**, what a Host's Invite Links open, with a manifest
  (`__tooob/version.json`) naming its Release, its Protocol Version and every file's hash.
- **`tooob --version`** prints the Release and the Protocol Version; a starting Host logs them.
