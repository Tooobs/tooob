# Changelog

Each Release of tooob: the `tooob` command, the Bootstrap Bundle and the Host's container image,
all with one Release number (ADR 0017). Release numbers follow [SemVer](https://semver.org); while
they're `0.x`, expect the commands to change. Add to "Unreleased" as part of each change;
`./release.sh` turns it into the next Release's section.

## Unreleased

### Added
- `tooob backup NAME FILE` saves one Host (its Host Key, Allowlist, Invites and Host Settings, not its usage counts or cookie jars) into one file sealed under a passphrase (argon2id and XChaCha20-Poly1305, ADR 0029), and `tooob restore NAME FILE` brings it back as the same Host, on this machine or another. Both take `--passphrase-file PATH`, or `-` for standard input. `restore` refuses a wrong passphrase, a taken name, and a Host Key another Host here already has, writing nothing (#5).
- `tooob restore NAME FILE --serves PATH` restores a Host whose folder (or web app) is somewhere else on this machine: PATH is checked as `tooob add` checks it, and the Host's record and settings both move to it, the settings' other lines kept. `tooob run` still refuses a Host whose `serves` was edited by hand, and now says to `tooob rm` it and restore it with `--serves` (#127).
- Enrollment Codes (ADR 0030): `tooob-relay code --label NAME [--uses N] [--expires 7d]` prints a code naming its Relay (`r.tooobs.net/7Q4K-M2XD-9PHT-WB3C`; one use, 7 days by default), `codes` lists the unused ones and `cancel <code or label>` cancels them. A Host connecting with a code as iroh's auth token is enrolled under its label and a use is spent; an expired, used up, cancelled or wrong code enrolls nothing and the Host is told which, and 5 wrong codes from one address refuse redemption from it for 15 minutes. Only a hash of each code is kept. The Host's `--relay-code` comes next (#129).

### Changed
- A Relay runs tooob-relay, iroh-relay 1.3.0 with a forwarding rule (ADR 0030): once its Enrolled Key list (`/data/tooob-relay/enrolled.toml`, edited with `tooob-relay enroll`, `remove` and `list`) has any key, it forwards only traffic from or to one, and a removed key is disconnected at once; an empty list leaves it open. `/__tooob/relay.json` says `{"relay_access":"open"}` or `"enrolled"`, and the log says which at start and on each change. Redeploy a Relay of your own to get it (#128).
- `./release.sh` runs the Host's tests on Linux (`test/linux/run.sh`, in podman) before it tags, and refuses when podman isn't running; CI runs `cargo test` on both musl targets on every push to `main` (#108).
- `tooob invites` shows when each Invite expires in words (`in 7 days`, `in 5 hours`, `in 40 minutes`), rounded up, instead of a Unix timestamp; `--json` still gives `expires_at` in Unix seconds (#3).

### Fixed
- A Relay sends the Bootstrap compressed: its image makes brotli (`-q 11`) and gzip (`-9`) copies of the Bundle's WASM, JS, HTML, CSS and JSON when it's built, and Caddy (now 2.11) serves them with `precompressed br gzip`, so the WASM goes from 3.1 MB on the wire to about 0.8 MB. The Bundle and its hashes don't change; iroh's paths aren't compressed. Redeploy a Relay of your own to get it (#113).
- A request the browser gives up on (the Visitor navigates away from a long-polling page) now ends the Host's request to the Site at once, even while the Site is sending nothing, on the Direct Path and through the Relay, so open requests no longer pile up on the Site (#2).

## 0.3.5 — 2026-11-07

Protocol Version: `tooob/0`. Tested with iroh-relay 1.3.0.

### Changed
- `tooob share --uploads`, and `tooob run` of a Host with uploads on, say "Visitors can add files to any folder in ~/drop, except hidden ones". `tooob ls` says when a Host's uploads are off and why, and `--json` adds `uploads_off` (#121).

### Fixed
- Uploads can no longer plant a file the Owner's machine runs. The Host refuses any upload that is hidden or goes into a hidden folder, asked for or reached through a symlink (`.git/hooks/pre-commit`, `.zshrc`, `.ssh/…`), and a hidden folder's listing has no Upload button. A Shared Folder that is the home folder, or holds it, takes none: `share --uploads` refuses it, and a Host set up so before starts read-only, saying why. An uploaded file is written 0644 at most, never executable, and on macOS it carries `com.apple.quarantine` naming tooob, so Gatekeeper checks it before it's opened (#121).

## 0.3.4 — 2026-11-05

Protocol Version: `tooob/0`. Tested with iroh-relay 1.3.0.

### Added
- A device waiting for approval (First-Visit Approval, or an Owner Invite) shows a Device Code, six digits; `tooob visitors` and the log show the same code, and `tooob approve NAME CODE` approves by it, so the Owner lets in the device they meant to (#110).
- Uploads into a Shared Folder, add-only and off by default: `tooob share --uploads` (or `uploads = true` in Host Settings) gives every allowed Visitor an Upload button and drag-and-drop in each folder's listing. Each file is a streamed `PUT`, saved whole or not at all, never overwriting (a taken name gets ` (1)`), refused for unsafe names, in a folder serving `index.html`, or when it would leave under 1 GiB free (`507`). Files in such a folder are served with `Content-Security-Policy: sandbox`, and the log records each finished upload (#116).

### Changed
- `tooob visitors` shows statuses in words ("waiting for approval"), with a CODE column; `--json` keeps its names and adds `device_code` (#110).

## 0.3.3 — 2026-11-01

Protocol Version: `tooob/0`. Tested with iroh-relay 1.3.0.

**0.3.3 is the first published 0.3 Release.** 0.3.0, 0.3.1 and 0.3.2 were tagged, but their builds
failed in CI (Linux tests, then GitHub retiring macOS 14, then a test racing on a slow machine),
so none was published: everything under 0.3.2, 0.3.1 and 0.3.0 below arrives with 0.3.3.

### Fixed
- A Host test that raced its own one-second Invite on a slow machine (#103).

## 0.3.2 — 2026-11-01

Protocol Version: `tooob/0`. Tested with iroh-relay 1.3.0.

**0.3.2 is the first published 0.3 Release.** 0.3.0 and 0.3.1 were tagged, but GitHub cancelled their
macOS builds (its macOS 14 machines are being retired), so neither was published: everything under
0.3.1 and 0.3.0 below arrives with 0.3.2.

### Fixed
- The macOS build runs on macOS 15, as GitHub retires macOS 14.

## 0.3.1 — 2026-11-01

Protocol Version: `tooob/0`. Tested with iroh-relay 1.3.0.

**0.3.1 is the first published 0.3 Release.** 0.3.0 was tagged, but its Linux builds failed in CI and it was
never published, so everything listed under 0.3.0 below arrives with 0.3.1.

### Fixed
- The Linux builds' terminal tests (#106): they assumed macOS's bash, and now pass on Linux as on macOS.
  tooob itself was unchanged.

## 0.3.0 — 2026-11-01

Protocol Version: `tooob/0`. Tested with iroh-relay 1.3.0.

### Added
- **Saving Shared Folder files:** a `⬇ Download` link on each file in a listing, a `⬇ Download`
  button and Cmd-S / Ctrl-S while a file is open, all saving the file itself. A Relay answers the
  browser's "Save link as" (which can't reach the Host) with a short note instead of an empty page
  (ADR 0023, amended).
- **The Host's log says who visits:** a Visitor getting in (first visit or return, and on which
  path) and leaving, moves between the Relay and the Direct Path, refused devices, the Owner's
  Invites, Blocks and approvals, and the Host's own starts and stops, each with an `event` and
  `fields` in `tooob logs --json`. Never pages, files or addresses (ADR 0025).
- **How much each Visitor uses:** a Visit's "left" line gives the bytes sent and received, and
  `tooob visitors` shows LAST SEEN, SENT and RECEIVED since each Visitor first got in, kept across
  restarts.
- **`tooob start NAME`** starts a Host that `tooob stop` kept stopped, kept running again; with no
  NAME it lists them. `tooob ls` shows such a Host's AUTOSTART as `stopped` (`kept_stopped` in
  `--json`).
- **A Host run in a terminal** prints its Invites in a block of their own, logs "still here" with
  the bytes so far every 10 minutes of a Visit, and takes two keys while it's the terminal's
  foreground job: `q` quits, `d` detaches (hands it to launchd or systemd, as `run -d`).
  `tooob invites` has a header and aligned columns.

### Changed
- **Invite Links that don't work say why:** a cut-short or damaged link says so (and never reaches
  the Host), and a used one says it may have been used in another browser, since each Invite opens
  the Site in one browser.
- **`run -d` and `share -d` replace `enable`,** and `stop` keeps a Host stopped, across logins and
  reboots, until it's started again; `rm` stops and removes a running or kept-running Host. `enable`
  and `disable` are removed: each says what to type instead (`tooob run NAME -d`, `tooob stop NAME`).
  Hosts enabled with 0.2.0 keep working (ADR 0014, amended).
- **A Host keeps its own log** in its state folder, readable only by the Owner, with a timestamp on
  every line, rolling over at about 1 MB into one older file. `tooob logs NAME` reads it for any
  Host (`-n N|all`, `-f`, `--json`); an enabled Host's crash output is shown and folded in at its
  next start (ADR 0025).

## 0.2.0 — 2026-10-31

Protocol Version: `tooob/0`. Tested with iroh-relay 1.3.0.

### Added
- **Autostart:** `tooob enable NAME` hands a Host to launchd (macOS) or systemd (Linux), which
  starts it now, at login and after a crash; `tooob disable NAME` removes the entry, `tooob logs
  NAME [-f]` shows its log, and `tooob stop` keeps an enabled Host stopped until the next login.
  `tooob ls` has an AUTOSTART column (`autostart` in `--json`). A serving Host keeps a Mac from
  idle-sleeping. Without systemd or launchd, `enable` says what to have your own service manager run.
- **`tooob upgrade`** installs the newest Release, or a named one, through that Release's own
  installer, after asking (`--yes` for scripts, `--check` to only look), and restarts enabled Hosts
  on it. It refuses an upgrade that would leave a Host on a Bootstrap Mismatch unless `--anyway`.
  Running Hosts check once a day and say when a newer Release is out; `TOOOB_NO_UPDATE_CHECK=1`
  turns that off (ADR 0024).

### Changed
- **`tooob` asks nothing.** No command asks a question, in a terminal or not, so `tooob share PATH
  --relay URL --invite NAME` is one shot that works in a script or backgrounded. A missing piece
  is a one-line error with the command to type, a placeholder in the gap. `share` makes exactly the
  Invites `--invite` names, and warns in a line, without stopping, when the folder holds `.git`,
  `.env` or `.ssh` or is the whole home folder (ADR 0022).

## 0.1.0 — 2026-10-30

Protocol Version: `tooob/0`. Tested with iroh-relay 1.3.0.

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
- **tooob checks its Relay's Bootstrap** (ADR 0020): `add`, `share` and `run` refuse a Relay whose
  Bootstrap speaks another Protocol Version, or that serves none, saying why and what to do; one
  that can't be reached stops `add` but not `run`. A running Host checks again every 10 minutes and
  keeps serving through a Bootstrap Mismatch, which its log, `tooob ls` and `tooob show` report. A
  starting Host logs its Relay's Bootstrap beside its own Release.
- **Uploads of any size** stream from the Visitor's browser to the Site, held whole by neither the
  browser nor the Host; the 64 MiB cap is gone. A body over 1 MiB reaches the Site chunked. On the
  Direct Path each request has its own flow control, so a big upload to a slow Site doesn't hold up
  the Visitor's other requests, and a Site that answers early stops the rest being sent.
- **`tooob --version`** prints the Release and the Protocol Version; a starting Host logs them.
