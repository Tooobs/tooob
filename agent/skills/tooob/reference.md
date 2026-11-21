# tooob: looking at and changing Hosts

NAME is a Host Name, what the Host serves, or a Host Key prefix. These commands work whether the Host is running or not. `tooob <command> --help` shows each one's usage.

## Looking

- `tooob ls --json`: every Host on this box, running or stopped: its `name`, `state`, what it `serves`, its `visitors`, and whether its Relay carries it (`enrollment`).
- `tooob show NAME`: its settings (with its `relay`), Host Key and files.
- `tooob visitors NAME --json`: its Visitors, when each was last seen, and what was sent to and received from them (`sent`, `received`). Both are bytes since the Visitor first got in, counted when the Host writes or reads them, not when the browser acknowledges them. `sent` counts every folder listing page, every file opened, and any retry or second download, but not the Bootstrap, which the Relay serves.
- `tooob invites NAME --json`: its unused Invites, with their ids.
- `tooob logs NAME --json`: its log, one object a line (`-n 200` or `-n all` for more; never `-f` from an agent, which follows forever).

## Changing

- `tooob approve NAME CODE`: let in the waiting device showing Device Code CODE, once the person reads you the same code.
- `tooob revoke NAME INVITE-ID`: an Invite not used yet stops working.
- `tooob block NAME KEY`: a Visitor's device is shut out (a unique prefix of its key is enough).
- `tooob stop NAME`: stop it; one kept running stays stopped until `tooob start NAME`.
- `tooob rm NAME`: stop it and delete it, Host Key and all.

Only for Hosts you made, unless the Owner says otherwise. Never `tooob unblock` unless the Owner asks.

## Commands that block

`tooob share` and `tooob run` without `-d` serve in the foreground until stopped: never run them in the foreground. Use `-d`, or run them in the background with stdout in a file.

## Exit codes

0 on success; 1 with a `tooob: …` line on stderr that says what's wrong and, mostly, the command to type instead. Warnings are `tooob: warning: …` lines on stderr.

## TOOOB_HOME

Where tooob keeps its Hosts (`~/.tooob` by default). Set it only to work on a throwaway set of Hosts, as the evals do.
