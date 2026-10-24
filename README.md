# tooob

A semi-private web. Run something at home (Home Assistant, Jellyfin, Immich, a folder of photos) and let the people you invite reach it from an ordinary browser: no port forwarding, no public URL, and nothing in between that can read the traffic.

A **Host** on your home box makes one **Site** reachable over [iroh](https://iroh.computer). A Visitor opens an **Invite Link**. Their browser loads a small static page, the **Bootstrap**, which connects to the Host through a **Relay** and then, when it can, straight to it over WebRTC. The Relay forwards encrypted traffic it can't read. Who may get in is decided only on the Host.

It's **private, not anonymous**: a Relay still sees which keys talk, when, how much and from where ([ADR 0007](docs/adr/0007-private-not-anonymous-no-public-ingress.md)).

**Status:** early. v1 is for technical early adopters on Linux and macOS. Expect the commands to change.

## What you need

tooob names no Relay or Bootstrap of its own ([ADR 0015](docs/adr/0015-no-default-relay-or-bootstrap.md)). Each Host's settings name both, so you run them or use ones run by someone you trust. `deploy/` has both:

- **A Relay**: stock [iroh-relay](https://github.com/n0-computer/iroh) 1.3.0. Use `deploy/relay/` (any VPS with Docker; `wizard.sh` walks through it) or `deploy/fly-relay/` (Fly.io). As shipped, a Relay lets anyone use it: see `access` in its config before you share it.
- **A Bootstrap**: `bootstrap/www`, built and served over HTTPS (browsers only run its service worker on a secure origin). Use `deploy/fly-bootstrap/`, or any static host.

For a try on one network only, skip the Relay with `--local`. You still need a Bootstrap.

## Build

```sh
cd host && cargo build --release            # the tooob command: host/target/release/tooob
cd ../bootstrap && ./build.sh               # the Bootstrap's WASM into bootstrap/www
```

`build.sh` needs the `wasm32-unknown-unknown` target, `wasm-bindgen-cli` at exactly the `wasm-bindgen` version in `bootstrap/web/Cargo.toml`, a clang that can target wasm (set `LLVM=` to its `bin/`; on macOS, Homebrew's `llvm`), and Node, which writes the Bootstrap Bundle's manifest, `www/__tooob/version.json`: its Release, its Protocol Version and every file's SHA-256. `node bootstrap/manifest.mjs --check bootstrap/www` says what no longer matches it, and `--hash` prints the Bundle's hash: its manifest's SHA-256.

## Use

```sh
tooob add photos ~/Photos --relay https://relay.example/ --bootstrap https://bootstrap.example/
tooob run photos                            # in the foreground; Ctrl-C ends it
tooob invite photos mom --expires 7d        # prints the Invite Link, and nothing else
tooob visitors photos                       # who has got in
tooob block photos <key prefix>
```

`TARGET` is a folder or file (served read-only, as a Shared Folder) or `HOST:PORT` for a web app. A Host's settings are a file you edit by hand (`tooob show photos` says where). Owner commands work whether the Host is running or not. `tooob` with no arguments lists every command.

## Develop

- `./check.sh`: formatting, clippy, `cargo test`, the Bootstrap's build and the Bootstrap Bundle's manifest tests; `git config core.hooksPath .githooks` (once per clone) runs it before every push.
- `cd bootstrap && ./build.sh && cd check && npm install && node --test`: end-to-end tests in headless Chrome against real Hosts and a local Relay (`cargo install iroh-relay --features server --version 1.3.0`).
- `GLOSSARY.md` is the vocabulary, `DESIGN.md` the plan, and `docs/adr/` the decisions that are hard to reverse. `CLAUDE.md` is how we work, with or without an agent.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option ([ADR 0016](docs/adr/0016-mit-or-apache-2.md)).

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
