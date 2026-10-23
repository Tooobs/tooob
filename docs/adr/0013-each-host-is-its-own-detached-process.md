# Each Host is its own process, which can detach from the terminal

_Partly superseded by ADR 0014: a Host no longer detaches itself or asks to; it runs in the foreground, and `tooob enable` hands it to the service manager. One process per Host still holds._

Each Host runs in its own process. In a terminal, `tooob share` and `tooob serve` ask, once the Owner has chosen what to share and who to invite, whether to keep the Host running in the background. `--detach` and `--foreground` answer that in advance. Without a terminal (Docker, a service manager, a script) they never ask and stay in the foreground unless told `--detach`, because those callers expect to supervise the process themselves. `tooob list` shows every Host on the box, running or stopped. From there the Owner can bring a running one back to the terminal (following it, then handing it back to the background), invite someone to it or end it, and start a stopped one again as it was.

We rejected one background process running every Host, which is what Docker or Tailscale do and what #11 planned. One process would need only one service entry, but ending or restarting one Site would mean going through that process, and a crash would take every Site down. With one process per Host, `ps` and `kill` work on one Site at a time. This replaces ADR 0009's note that several Sites could run under one tooob process.

## Consequences

- Each Host has its own data directory under one parent, so `tooob list` and the Owner's commands find a Host by name. The directory records what the Host serves and on which network, so a stopped Host starts again as it was, and never for another Site (ADR 0009).
- Starting at login or after a crash needs one service entry per Host.
- "Bringing a Host back" means following its activity and talking to it over its control socket. A detached process can't be re-parented to a shell, as a shell job's `fg` would be.
