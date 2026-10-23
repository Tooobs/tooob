# tooob

A semi-private web: a Host on someone's home box makes a Site reachable to invited Visitors in ordinary browsers, over iroh, through Relays that can't read the traffic.

## Language and decisions

- `GLOSSARY.md` is the vocabulary. Name code, tests and commits in its terms; settle a new or fuzzy term there first (`domain-modeling` skill).
- `docs/adr/` holds the hard-to-reverse decisions. Read the ADR for an area before changing it.
- `DESIGN.md` is the product plan and scope (v1, v1.1, v2).

## Where things live

- `host/`: the v1 crate, where product code goes: the Host, the `tooob` CLI, `cargo test`.
- `bootstrap/`: the v1 browser side: the Bootstrap, what a Host's Invite Links open. `./build.sh`, then `cd check && node --test`: end-to-end tests in headless Chrome against real Hosts (`tooob add` + `tooob run`) and a local Relay (needs `cargo install iroh-relay --features server --version 1.3.0`).
- `spike/`: throwaway proofs, kept as reference and as the source to port from. Run steps, measurements and findings are in `spike/README.md`.
- `deploy/`: running your own Relay and Bootstrap, since tooob names none (ADR 0015). `relay/` is a VPS with Docker, plus a wizard; `fly-relay/` and `fly-bootstrap/` are Fly.io apps.

## How we work

- **Grill, doc, ticket, then build.** When the user asks for something new or changed (a feature, a behaviour, a design), no code until: (1) `grilling` on it; (2) `domain-modeling` writes what was settled into `GLOSSARY.md` and, if it qualifies, an ADR; (3) the outcome is filed as a GitHub issue, or written into an existing one, with a milestone; (4) only then `implement` that issue. Decisions that come up while building go back into the issue before the code. If the user asks to skip a step, name the step being skipped and have them confirm it; never skip one silently. Only bug reports skip grilling: they go through `diagnosing-bugs`, then an issue.
- **TDD at agreed seams.** Propose the seams and confirm them with the user, then go red → green one test at a time through the public interface (`tdd` skill).
- **Close each finished piece of work by recommending the next mattpocock-skills steps:** `code-review` after `tdd`, `diagnosing-bugs` before chasing a bug, `research` to gather facts into the repo.

## Agent skills

### Issue tracker

GitHub Issues on Tooobs/tooob (via `gh`), filed under milestones v1, Normal people, v2 and Sync. See `docs/agents/issue-tracker.md`.

### Triage labels

The five default triage labels (needs-triage, needs-info, ready-for-agent, ready-for-human, wontfix). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

## Gotchas

- `wasm-bindgen-cli` must be exactly the `wasm-bindgen` version in `bootstrap/web/Cargo.toml` and `spike/web/Cargo.toml` (0.2.129).
- The spike's Bootstrap uses port 8088.
- Check browsers with headless Chrome (`spike/check/*.mjs`, Puppeteer). Automation tabs in your everyday Chrome are hidden, and hidden tabs never load media.
- Test harnesses that start HTTP servers must close them, or `node --test` never exits.
- Start long-running servers detached with `< /dev/null` and output redirected; an inherited pipe keeps the calling command from ever returning.
- The Direct Path paces sends (80 Mbit/s default, `TOOOB_PACE_MBPS`): unpaced bursts into Chrome stall SCTP for 1–13 s (webrtc-rs/webrtc#938). Re-run the stall test before raising it.
- Measure speed on a Relay of your own; n0's community relays are throttled.
