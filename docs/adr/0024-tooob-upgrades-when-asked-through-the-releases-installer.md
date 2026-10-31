# tooob upgrades when asked, through the Release's own installer

`tooob upgrade` replaces the `tooob` command with another Release, and only when the Owner runs it: nothing upgrades by itself. It shows the newest final Release (never a test Release) with a link to its notes and asks before installing (`--yes` for scripts, `--check` to only look, `tooob upgrade VERSION` for an exact Release, older ones included). It installs by running that Release's own `tooob-installer.sh`, fetched by its tag, the same path an Owner takes by hand, into `~/.local/bin/tooob`, the path the service manager's entries run (ADR 0014). Enabled Hosts are restarted through the service manager; Hosts run by hand keep the old Release until they're restarted, and `upgrade` names them. Running Hosts look for a newer Release at most once a day and say so in their log and in `tooob ls`; `TOOOB_NO_UPDATE_CHECK=1` turns that off.

We rejected upgrading automatically: a home box would change under its Owner unseen, and a Release that changes the Protocol Version would take every Host down until its Relay serves the new Bootstrap (ADR 0017, ADR 0020). So `upgrade` checks each Host's Relay first and refuses an upgrade that would leave a Host on a Bootstrap Mismatch, naming the Hosts and Relays, unless `--anyway` says so. We also rejected tooob downloading and swapping its own binary: it would be a second install path, beside the installer, to keep correct.

## Consequences

- `upgrade` is the one command that asks a question; ADR 0022 is amended to say so. Without a terminal it refuses unless `--yes`.
- `upgrade` only replaces `~/.local/bin/tooob`. A `tooob` elsewhere (built from source, moved) or in a container is refused, saying what to do instead (run the installer, pull the new image).
