# tooob's plugin for agents

A Claude Code plugin holding one skill, `tooob` (`skills/tooob/SKILL.md`, in the open Agent Skills format), that teaches an agent to share and receive files with the `tooob` CLI: an agent in the Owner's own account, with every Owner power (ADR 0018).

```
/plugin marketplace add Tooobs/tooob
/plugin install tooob@tooob
```

The marketplace (`.claude-plugin/marketplace.json` at the repo root) pins the plugin to a Release's tag, and `plugin.json`'s `version` is that Release: `release.sh` sets both. A `cargo test` in `host/` checks that every `tooob …` command the skill names is one the CLI has.

## The eval

One case, `evals/send-a-file`: "Send report.pdf to Sam". It checks that the skill fired, that `tooob share … -d --expires 1d --no-qr` ran, and that the reply holds an Invite Link (`#invite=`). It makes real model calls (6 runs) and a real Host on r.tooobs.net, so it isn't in CI or `release.sh`: run it by hand before a Release that changes the skill, while r.tooobs.net is open (the g14's test Relay has no domain, so it makes no Invite Links).

From the repo root, with the `tooob` under test on `PATH`:

```
claude plugin eval agent --scaffold --allow-tools Bash --no-publish
```

Each run has a home folder of its own, so its Hosts go in a throwaway `~/.tooob`. Afterwards, check that no Host the eval made is still running (`ps aux | grep 'tooob '`, or `tooob ls` under that home if `--keep-temp` kept it) and stop any that is.
