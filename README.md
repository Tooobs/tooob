# tooob

A semi-private web for people and their AI agents. Run something at home, or have your agent run it (Home Assistant, a folder of photos, a status dashboard, a dev preview), and let the people and agents you invite reach it in an ordinary browser: no port forwarding, no public URL, and nothing in between that can read the traffic.

A **Host** on your home box makes one **Site** reachable over [iroh](https://iroh.computer). A Visitor opens an **Invite Link**. Their browser loads a small static page, the **Bootstrap**, which connects to the Host through a **Relay** and then, when it can, straight to it over WebRTC. The Relay forwards encrypted traffic it can't read. Who may get in is decided only on the Host.

It's **private, not anonymous**: a Relay still sees which keys talk, when, how much and from where ([ADR 0007](docs/adr/0007-private-not-anonymous-no-public-ingress.md)).

**Status:** early. v1 is for technical early adopters on Linux and macOS. Expect the commands to change. [What you can try](#what-you-can-try) says what works so far, and [the roadmap](docs/roadmap.md) what comes next.

## What you need

**A Relay.** tooob names none of its own ([ADR 0015](docs/adr/0015-no-default-relay-or-bootstrap.md)), so you run one, or use one run by someone you trust. A Relay serves the Bootstrap too, so it's all a Host needs. [`deploy/`](deploy/README.md) has two setups, a VPS with Docker (`wizard.sh` walks through it) and Fly.io, and lists what a Relay needs: a domain of its own with a wildcard DNS record, an API token for that DNS, TCP 443 and generous bandwidth. As shipped, a Relay lets anyone use it: see `access` in its config before you share it.

## Install

On macOS (Apple Silicon) or Linux (arm64, x86_64), a prebuilt `tooob`:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Tooobs/tooob/releases/latest/download/tooob-installer.sh | sh
```

It goes in `~/.local/bin`: open a new terminal, then `tooob --version`. To upgrade, run it again. Every Release's archives, checksums and notes are on [Releases](https://github.com/Tooobs/tooob/releases). The macOS binary isn't signed yet ([#52](https://github.com/Tooobs/tooob/issues/52)), so one downloaded in a browser won't run until you clear its quarantine: `xattr -d com.apple.quarantine tooob`.

## Build from source

```sh
cargo build --release                       # the tooob command: target/release/tooob
bootstrap/build.sh                          # the Bootstrap's WASM into bootstrap/www
```

`build.sh` needs the `wasm32-unknown-unknown` target, `wasm-bindgen-cli` at exactly the `wasm-bindgen` version in `bootstrap/web/Cargo.toml`, a clang that can target wasm (set `LLVM=` to its `bin/`; on macOS, Homebrew's `llvm`), and Node, which writes the Bootstrap Bundle's manifest, `www/__tooob/version.json`: its Release, its Protocol Version and every file's SHA-256. `node bootstrap/manifest.mjs --check bootstrap/www` says what no longer matches it, and `--hash` prints the Bundle's hash: its manifest's SHA-256.

## Use

```sh
tooob add photos ~/Photos --relay https://relay.example/
tooob run photos                            # in the foreground; Ctrl-C ends it
tooob invite photos mom --expires 7d        # prints the Invite Link (https://<host key>.relay.example/#invite=…)
tooob visitors photos                       # who has got in
tooob block photos <key prefix>
```

`TARGET` is a folder or file (served read-only, as a Shared Folder) or `HOST:PORT` for a web app. A Host's settings are a file you edit by hand (`tooob show photos` says where). Owner commands work whether the Host is running or not. `tooob` with no arguments lists every command.

An Invite Link, piece by piece ([ADR 0020](docs/adr/0020-a-relay-serves-its-bootstrap.md)):

```
https://kgjwugk89iaan7dtoyuju7pts5r7j3f1zxm3zp8k98nepg1hq46y.relay.example/#invite=00be5118…
        └──────────────────── Host Key ────────────────────┘ └── Relay ──┘  └── Invite ──┘
```

- **The Host Key**: the Host's identity and its address, made with the Host (`tooob show` prints it). It's in z-base-32, 52 characters, so it fits in a hostname. It isn't secret: who gets in is decided on the Host.
- **The Relay's domain**: the visitor's browser loads the Bootstrap from it, and reaches the Host through it.
- **Together, the Key Address** (`<host key>.<relay domain>`): each Host is an origin of its own, so no Site's scripts can reach another's browser storage or Visitor Key. It lasts as long as the Host Key and the Relay: a new Relay, or a deleted Host, means new Invites for everyone.
- **After `#`, the Invite**: browsers never send this part to a server, so the Relay never sees it. It works once, for one device, and the page drops it from the address bar once it's used.

Each of a Site's pages has its own address on the Key Address: `https://<host key>.<relay domain>/subfolder/` opens that page ([ADR 0023](docs/adr/0023-site-pages-have-real-paths-on-their-key-address.md)). Links within a Site work as they would anywhere, and the address bar and the tab's title follow the Site, so a reload, a bookmark or a shared link lands on the page it was.

## What you can try

What a technical early adopter can try after each push, in the order it landed. Each entry is a ticket, the date its work was pushed, and the first Release that has it. A finished milestone's summary says what it adds up to.

### [v1 · Technical early adopters](https://github.com/Tooobs/tooob/milestone/1) (in progress)

Everything here is in the first Release, **0.1.0 (not out yet)**: until then, the test Releases on [Releases](https://github.com/Tooobs/tooob/releases) have it. Tickets finished before 2026-10-23 came over from an earlier tracker that day, so their own dates are lost.

- **A Site on a phone, in an ordinary browser** ([#37](https://github.com/Tooobs/tooob/issues/37), [#40](https://github.com/Tooobs/tooob/issues/40), [#50](https://github.com/Tooobs/tooob/issues/50)) · before 2026-10-23 · 0.1.0 (not out yet)
  - Run a Relay of your own ([`deploy/`](deploy/README.md)): tooob has no default one. Then `tooob add photos ~/Pictures --relay https://<your relay>/`, `tooob run photos`, and in another terminal `tooob invite photos phone`. Open the link on a phone: the Site loads, and the pill in the corner says how fast it connected and which path it came by.
- **Straight to the Host when it can** ([#38](https://github.com/Tooobs/tooob/issues/38), [#39](https://github.com/Tooobs/tooob/issues/39)) · before 2026-10-23 · 0.1.0 (not out yet)
  - With the Site open, the pill says `direct` when the browser reaches the Host over WebRTC, and whether that's on the home network or punched through NAT from elsewhere. Otherwise it says `relay`. Try it on Wi-Fi at home, then on cellular. The Direct Path carries on as a phone switches apps or locks its screen for a while; after a long background it may come back on the Relay instead ([#15](https://github.com/Tooobs/tooob/issues/15)).
- **A folder or a file as a Site** ([#44](https://github.com/Tooobs/tooob/issues/44)) · before 2026-10-23 · 0.1.0 (not out yet)
  - `tooob share ~/Pictures --relay https://<your relay>/`, or `tooob add` with a folder or file. Visitors browse the folder and its subfolders, open photos in the browser, and download what it can't show under the file's own name. It's read-only.
- **Staying logged in** ([#41](https://github.com/Tooobs/tooob/issues/41)) · before 2026-10-23 · 0.1.0 (not out yet)
  - Share a web app with a login (`tooob add NAME HOST:PORT`), log in as a Visitor, then restart the Host: the Visitor is still logged in. Each Visitor's cookies for the Site are kept on the Host, never in their browser.
- **The Visitor Key stays the browser's** ([#43](https://github.com/Tooobs/tooob/issues/43)) · before 2026-10-23 · 0.1.0 (not out yet)
  - Nothing shows: the key that proves a Visitor's device is kept so that the Site's own scripts can't read it out.
- **Knowing when the Host is away** ([#45](https://github.com/Tooobs/tooob/issues/45), [#46](https://github.com/Tooobs/tooob/issues/46)) · before 2026-10-23 · 0.1.0 (not out yet)
  - Stop the Host (Ctrl-C) while a Visitor has the Site open. Their pill says `Reconnecting…` at once and counts its tries. Start the Host again and the Site comes back by itself; leave it stopped and the pill offers a reload instead.
- **Hosts from the command line** ([#47](https://github.com/Tooobs/tooob/issues/47), [#48](https://github.com/Tooobs/tooob/issues/48), [#49](https://github.com/Tooobs/tooob/issues/49)) · before 2026-10-23 · 0.1.0 (not out yet)
  - `tooob add` makes a Host once, `tooob run` serves it in the foreground until Ctrl-C, and `tooob ls`, `show` and `rm` list, describe and delete Hosts. Owner commands name the Host first (`tooob invite photos mom`), work whether it's running or not, and a command that changes something prints nothing when it works. [#47](https://github.com/Tooobs/tooob/issues/47)'s background Hosts and `tooob list` were replaced by these ([ADR 0014](docs/adr/0014-hosts-run-in-the-foreground-the-os-keeps-them-running.md)).
- **[#56](https://github.com/Tooobs/tooob/issues/56) `tooob --version`, and a Bootstrap Bundle says its Release and Protocol Version** · 2026-10-24 · 0.1.0 (not out yet)
  - `tooob --version` prints the Release and the Protocol Version (`tooob 0.1.0-rc.6 (tooob/0)`), and a starting Host logs them. On any Key Address, `/__tooob/version.json` names the Bundle's Release, its Protocol Version and every file's hash.
- **[#58](https://github.com/Tooobs/tooob/issues/58) Local checks run on every push** · 2026-10-24 · 0.1.0 (not out yet)
  - For contributors: after `git config core.hooksPath .githooks` (once per clone), `git push` runs `./check.sh` first, and a failing check stops the push and says which.
- **[#62](https://github.com/Tooobs/tooob/issues/62) The Host refuses a header line over its 64 KiB limit, however it arrives** · 2026-10-24 · 0.1.0 (not out yet)
  - Nothing shows: one that arrived in a single read used to get through.
- **[#65](https://github.com/Tooobs/tooob/issues/65) `tooob rm` works while other Owner commands run** · 2026-10-25 · 0.1.0 (not out yet)
  - Nothing shows unless you script Owner commands: `rm` could fail on a stray lock file, or mistake another command for a running Host.
- **[#66](https://github.com/Tooobs/tooob/issues/66) Key Addresses: each Host is its own origin** · 2026-10-26 · 0.1.0 (not out yet)
  - `tooob invite` prints `https://<host key>.<relay domain>/#invite=…`: the Host's own address on its Relay's domain, so one Site's scripts can't reach another's browser storage or Visitor Key. A Host names only its Relay, which serves the Bootstrap on its domain and every Key Address under it (the Relay setups in [`deploy/`](deploy/README.md), [#67](https://github.com/Tooobs/tooob/issues/67)).
- **[#78](https://github.com/Tooobs/tooob/issues/78) A Relay is safe for its Owner without the Public Suffix List** · 2026-10-28 · 0.1.0 (not out yet)
  - A Relay of your own doesn't need the list, and [`deploy/README.md`](deploy/README.md) says when one shared with others does. `curl -sI https://<host key>.<your relay>/` shows `Origin-Agent-Cluster` and `Cross-Origin-Opener-Policy`. A cookie a Site sets for the whole Relay domain stays with that Site.
- **[#82](https://github.com/Tooobs/tooob/issues/82) The browser suite runs on Playwright, and can't take over the machine** · 2026-10-29 · 0.1.0 (not out yet)
  - For contributors: `npx playwright install chromium` once in `bootstrap/check`, then `npx playwright test` there runs the browser suite in headless Chromium (`HEADED=1` opens windows you can watch). A second run started meanwhile refuses, a test that hangs fails at 90 s, and a run killed halfway leaves no Host or Relay running: the next run stops them first.
- **[#77](https://github.com/Tooobs/tooob/issues/77) A page's own address opens that page, and the address bar follows the Site** · 2026-10-29 · 0.1.0 (not out yet)
  - Open a Site and click into a subfolder: the address bar shows that page's path, and the tab shows its title, as the page changes it too (an unread count). Reload, or copy the address to another device that's a Visitor: it opens on the same page. An Invite Link works at a page's address too (`https://<host key>.<your relay>/subfolder/#invite=…`).

## Still to come

What each later milestone will add when it's done, in the planned order. Details and progress: [the roadmap](https://github.com/Tooobs/tooob/blob/main/docs/roadmap.md).

### [Agents](https://github.com/Tooobs/tooob/blob/main/docs/roadmap.md#agents)

AI agents run Hosts for their Owners and visit Sites in a browser like anyone else. An agent shares a status dashboard or a preview with people and other agents by invitation, and every device its Invites let in is approved by a person first ([ADR 0018](docs/adr/0018-agents-make-approve-first-invites-enforced-by-agent-accounts.md)). A local MCP server lets an agent read, block and revoke; anything that would let someone in waits for the Owner.

### [Relay finder](https://github.com/Tooobs/tooob/blob/main/docs/roadmap.md#relay-finder)

Owners find a Relay to use instead of running their own: a plain list anyone can host, or a trusted site with each Relay's stats and terms, and an audit that checks what each one serves against tooob's published Releases. Choosing stays the Owner's.

### [Normal people](https://github.com/Tooobs/tooob/blob/main/docs/roadmap.md#normal-people)

tooob for people who don't open a terminal: a signed `tooob` for every Mac and for Windows that upgrades itself, set-up that walks them through it, and Visitors who keep working when a Host and its Relay's Bootstrap are a Protocol Version apart. Every v1 choice is already checked against it.

### [v2](https://github.com/Tooobs/tooob/blob/main/docs/roadmap.md#v2)

A native visitor app or browser extension, which reaches Hosts with iroh's own hole punching, friendly names in place of Host Keys, and linking a person's devices. Perhaps native phone apps too, so a phone can be a Host and a Site goes phone to phone.

### [Sync](https://github.com/Tooobs/tooob/blob/main/docs/roadmap.md#sync)

A folder kept in step across the Owner's devices and the people they invite, like Dropbox: a change made on any of them reaches the others. Its scope is still to be settled.

## Develop

- `./check.sh`: formatting, clippy, `cargo test`, the Bootstrap's build, the Bootstrap Bundle's manifest tests and `release.sh`'s tests (which need `npm ci` in `bootstrap/check` once); `git config core.hooksPath .githooks` (once per clone) runs it before every push.
- `./release.sh X.Y.Z`: cuts a Release from a clean, up-to-date `main` (checks, the browser suite, the cross-Release check, then the Release number in both `Cargo.toml`s, `CHANGELOG.md`, a commit and its tag, pushed); CI then builds and publishes it. Add what each change does to `CHANGELOG.md`'s "Unreleased" as you go.
- The browser suite: end-to-end tests against real Hosts and a local Relay (`cargo install iroh-relay --features server --version 1.3.0`), on [Playwright](https://playwright.dev), in Chromium, headless unless `HEADED=1`. Once: `cd bootstrap/check && npm ci && npx playwright install chromium`. Then `bootstrap/build.sh`, and in `bootstrap/check`, `npx playwright test` (one test: `-g '<name>'`; `HEADED=1 SLOWMO=500` to follow along in windows). A run can't take over the machine: one at a time, one worker, at most 4 tabs, 90 s a test and 15 minutes a run, and no Host or Relay it started outlives it, even if it's killed. `node --test guards.test.mjs` there tests those guards.
- `GLOSSARY.md` is the vocabulary, `DESIGN.md` the plan, and `docs/adr/` the decisions that are hard to reverse. `CLAUDE.md` is how we work, with or without an agent.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option ([ADR 0016](docs/adr/0016-mit-or-apache-2.md)).

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
