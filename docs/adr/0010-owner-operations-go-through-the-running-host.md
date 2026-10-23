# Owner operations go through the running Host

_Amended by ADR 0014: when the Host isn't running, Owner commands change its state directly, while a running Host holds a lock on that state._

The running Host owns its access state. Every Owner operation (invite, pair, block, approve, list, revoke) is a request to that Host: the `tooob` CLI sends it over a local Unix socket, and Owner Devices send it over tooob itself. We rejected letting the CLI edit the access file directly: simpler, but a CLI and a running Host writing the same file would silently overwrite each other's changes. Pairing Invites are only issued over the local socket, so "physical access counts as ownership" is enforced by the operating system rather than by convention.
