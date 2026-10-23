# Hosts run in the foreground; the operating system keeps them running

A Host is made once (`tooob add NAME TARGET`) and then run (`tooob run NAME`). It always runs in the foreground: it logs to stderr, and ends cleanly on Ctrl-C or a signal. Keeping it running after the terminal closes, and across logouts, reboots and crashes, is the operating system's service manager's job: `tooob enable NAME` writes a launchd agent (macOS) or a systemd user unit (Linux) for that one Host, and `tooob disable NAME` removes it. This replaces ADR 0013's self-detach, and the question it asked in a terminal ("Keep sharing in the background?"). Each Host is still its own process (ADR 0013).

We rejected tooob detaching itself. A self-detached Host survives a closed terminal but not a reboot or a crash, so the service manager is needed anyway, and having two ways to run in the background means `stop` and `ls` have to know about both. A program that backgrounds itself also fights everything that expects to supervise it (a shell's `&`, tmux, Docker, a Home Assistant add-on, launchd, systemd). And "bringing a Host back to the terminal" was never real: a detached process can't be handed back to a shell.

A Host's settings (what it serves, its Relay, its Bootstrap) live in a file the Owner can edit, one per Host, apart from its state (Host Key, Allowlist, Invites, cookie jars), which only tooob writes. `run` takes no settings of its own, so a Host started by hand and the same Host started by the service manager behave the same.

## Consequences

- This amends ADR 0010. Owner commands still go through the Host when it's running. When it isn't, they change its state directly, so Invites can be made before a Host's first run. A running Host holds a lock on its state, which keeps a command from writing alongside it: the overwrite ADR 0010 rejected. Pairing Invites still need access to the home box itself.
- `stop` on an enabled Host goes through the service manager. Otherwise the service manager would start the Host again straight away.
- `enable` has nothing to write on Windows until #41.
