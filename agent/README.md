# tooob's plugin for agents

A Claude Code plugin holding one skill, `tooob` (`skills/tooob/SKILL.md`, in the open Agent Skills format), that teaches an agent to share and receive files with the `tooob` CLI: an agent in the Owner's own account, with every Owner power (ADR 0018).

```
/plugin marketplace add Tooobs/tooob
/plugin install tooob@tooob
```

The marketplace (`.claude-plugin/marketplace.json` at the repo root) pins the plugin to a Release's tag, and `plugin.json`'s `version` is that Release: `release.sh` sets both. Between Releases, `plugin.json`'s `version` stays `host/Cargo.toml`'s, so the skill's version check passes for an agent running it from `main`. A `cargo test` in `host/` checks both: that the two versions agree, and that every `tooob …` command the skill names is one the CLI has.

## The eval

One case, `evals/send-a-file`: "Send report.pdf to Sam". It checks that the skill fired, that `tooob share … -d --expires 1d --no-qr` ran, and that the reply holds an Invite Link (`#invite=`). It makes real model calls (6 runs) and a real Host on r.tooobs.net, so it isn't in CI or `release.sh`: run it by hand before a Release that changes the skill, while r.tooobs.net is open (the g14's test Relay has no domain, so it makes no Invite Links).

It tests the `tooob` built from `main`, the CLI the skill describes, not an installed Release. From the repo root, build it and put it first on `PATH`:

```
cargo build --manifest-path host/Cargo.toml
PATH="$PWD/target/debug:$PATH" claude plugin eval agent --scaffold --allow-tools Bash --no-publish
```

Build it again after each change you want the eval to see: the scaffold checks where `tooob` is, not how old it is. It refuses a run whose `tooob` isn't a `target/debug/tooob` (a link to one counts as the link's own path, so it's refused too), so the eval can't quietly test the installed one. Each run's report goes in `evals/results/`, which git ignores.

`tooob share -d` hands the Host to a systemd or launchd service that runs `~/.local/bin/tooob`, and a run's own home folder has none, so there `-d` fails and the skill falls back to a background `share`: that's expected, and the grader still sees the `share … -d` command it tried first.

Each run has a home folder of its own, so its Hosts go in a throwaway `~/.tooob`. Afterwards, check that no Host the eval made is still running (`ps aux | grep 'tooob '`, or `tooob ls` under that home if `--keep-temp` kept it) and stop any that is.
