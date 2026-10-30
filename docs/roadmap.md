# Roadmap

Every milestone's features and where each stands. The why and the scope of v1 to v2 are in [`DESIGN.md`](../DESIGN.md) (Sync's are still to be settled); what you can try so far is in [the README](../README.md#what-you-can-try). Work is tracked in [GitHub Issues](https://github.com/Tooobs/tooob/issues).

Status: ✅ done · 🟡 partly done · ⬜ not started · ➖ not applicable.

## Standing requirements

These hold for every milestone; changes must keep them.

- **Every v1 choice is checked against Normal people**: technical early adopters come first, but nothing v1 does may block people who never open a terminal.
- **Private, not anonymous** ([ADR 0007](adr/0007-private-not-anonymous-no-public-ingress.md)): nothing in between can read the traffic, and who gets in is decided only on the Host. A Relay still sees which keys talk, when, how much and from where.

## [v1 · Technical early adopters](https://github.com/Tooobs/tooob/milestone/1)

A home box serving a Site to invited Visitors in ordinary browsers, for technical early adopters on Linux and macOS.

### Reaching a Site

- ✅ A Visitor opens an Invite Link in an ordinary browser, on a phone too, and the Site loads ([#37](https://github.com/Tooobs/tooob/issues/37), [#40](https://github.com/Tooobs/tooob/issues/40))
- ✅ Straight to the Host over WebRTC when it can, the Relay when not, on real phones ([#38](https://github.com/Tooobs/tooob/issues/38), [#39](https://github.com/Tooobs/tooob/issues/39))
- ⬜ The Direct Path comes back after a long background or a 5G → Wi-Fi switch ([#15](https://github.com/Tooobs/tooob/issues/15))
- ✅ Visitors stay logged in across a Host restart ([#41](https://github.com/Tooobs/tooob/issues/41))
- ✅ The Visitor Key can't be read by the Site's own scripts ([#43](https://github.com/Tooobs/tooob/issues/43))
- ✅ Visitors learn at once when the Host stops, and the pill reconnects or offers a reload ([#45](https://github.com/Tooobs/tooob/issues/45), [#46](https://github.com/Tooobs/tooob/issues/46))
- ✅ A page's own address opens that page, and the address bar follows the Site ([#77](https://github.com/Tooobs/tooob/issues/77))
- ⬜ A Site's link to another Key Address opens that Site ([#80](https://github.com/Tooobs/tooob/issues/80))
- 🟡 Several tabs each reach their own Site; with several open at once, a request can now and then reach another tab's ([#11](https://github.com/Tooobs/tooob/issues/11))
- 🟡 Large uploads reach the Site, streamed, of any size ([#1](https://github.com/Tooobs/tooob/issues/1)); a cancelled request stops the Host's request to it ([#2](https://github.com/Tooobs/tooob/issues/2))
- ⬜ A cut-short Invite Link says so ([#32](https://github.com/Tooobs/tooob/issues/32)); a browser that can't open a tooob link is told why ([#14](https://github.com/Tooobs/tooob/issues/14)); a Bootstrap and a Host on different Protocol Versions say so ([#55](https://github.com/Tooobs/tooob/issues/55))

### Sharing and the `tooob` command

- ✅ A folder or a file is a Site, no web app needed ([#44](https://github.com/Tooobs/tooob/issues/44))
- ✅ `tooob add`, `run`, `ls`, `show` and `rm`; Owner commands name the Host first and work with it stopped ([#47](https://github.com/Tooobs/tooob/issues/47), [#48](https://github.com/Tooobs/tooob/issues/48), [#49](https://github.com/Tooobs/tooob/issues/49))
- 🟡 `tooob` works like a unix tool ([#33](https://github.com/Tooobs/tooob/issues/33)); the docs describe it all ([#34](https://github.com/Tooobs/tooob/issues/34))
- ✅ `enable`, `disable`, `stop` and `logs`: the service manager keeps a Host running across logins, crashes and idle sleep ([#27](https://github.com/Tooobs/tooob/issues/27))
- 🟡 `tooob share PATH` is add, invite and run in one ([#22](https://github.com/Tooobs/tooob/issues/22))
- ✅ `tooob` asks nothing, and shows the command to type instead; `share` warns about what a folder holds ([#79](https://github.com/Tooobs/tooob/issues/79))

### Visitors and the Owner

- ✅ Invites for one device each, with expiry or First-Visit Approval; Pairing Invites; block, approve, unblock and revoke (before the issue tracker)
- ⬜ An Invite can be reusable ([#18](https://github.com/Tooobs/tooob/issues/18)), and `tooob invites` shows readable expiry times ([#3](https://github.com/Tooobs/tooob/issues/3))
- ⬜ The Owner sees how much each Visitor uses ([#4](https://github.com/Tooobs/tooob/issues/4)), and a log of who got in, who was refused and what the Owner changed ([#23](https://github.com/Tooobs/tooob/issues/23))
- ⬜ The Owner sets the Host's bandwidth, and the Direct Path isn't stuck at 80 Mbit/s ([#21](https://github.com/Tooobs/tooob/issues/21))
- ⬜ A relay-only Host keeps the home IP hidden ([#25](https://github.com/Tooobs/tooob/issues/25))
- ⬜ The Host Key and Allowlist can be backed up and restored ([#5](https://github.com/Tooobs/tooob/issues/5))
- ⬜ The owner page: managing Visitors from a web page ([#7](https://github.com/Tooobs/tooob/issues/7)), first run pairing the Owner's browser from the home network ([#8](https://github.com/Tooobs/tooob/issues/8)), Owner Devices managing the Host over tooob ([#6](https://github.com/Tooobs/tooob/issues/6)), who's connected now and the unused Invites ([#26](https://github.com/Tooobs/tooob/issues/26)), bandwidth limits ([#24](https://github.com/Tooobs/tooob/issues/24))

### Relays and the Bootstrap

- ✅ No default Relay: every Host's settings name one ([#50](https://github.com/Tooobs/tooob/issues/50))
- ✅ Key Addresses: each Host is its own origin on its Relay's domain ([#66](https://github.com/Tooobs/tooob/issues/66))
- ✅ A Relay is safe for its Owner without the Public Suffix List ([#78](https://github.com/Tooobs/tooob/issues/78))
- 🟡 A Relay serves its Bootstrap: the Relay setups, checked on Fly.io, not yet on a VPS ([#67](https://github.com/Tooobs/tooob/issues/67))
- 🟡 tooob checks its Relay's Bootstrap: it won't start on a Bootstrap Mismatch, and warns of one while running ([#68](https://github.com/Tooobs/tooob/issues/68)); a switch of Relay is announced ([#69](https://github.com/Tooobs/tooob/issues/69))
- ⬜ `--local` is only for tooob's own tests ([#70](https://github.com/Tooobs/tooob/issues/70))
- ⬜ Hosts publish their Host Key and addresses to n0's DNS ([#36](https://github.com/Tooobs/tooob/issues/36))
- ⬜ Idea: an "only on this network" mode for browsers ([#71](https://github.com/Tooobs/tooob/issues/71))

### Releases and packaging

- ✅ `tooob --version`, and a Bundle names its Release and Protocol Version ([#56](https://github.com/Tooobs/tooob/issues/56))
- ✅ Releases: one command cuts one and CI publishes it; 0.1.0 is out (2026-10-30) ([#59](https://github.com/Tooobs/tooob/issues/59))
- ⬜ Anyone can rebuild a Release's Bundle and get the same bytes ([#60](https://github.com/Tooobs/tooob/issues/60))
- ⬜ The Host ships as a Docker image ([#12](https://github.com/Tooobs/tooob/issues/12)) and a Home Assistant add-on ([#13](https://github.com/Tooobs/tooob/issues/13))
- ✅ Local checks run on every push ([#58](https://github.com/Tooobs/tooob/issues/58))
- ✅ What you can try, and this roadmap ([#61](https://github.com/Tooobs/tooob/issues/61))

### Hardening and tests

- ✅ The Host refuses a header line over 64 KiB however it arrives ([#62](https://github.com/Tooobs/tooob/issues/62)); `tooob rm` survives Owner commands racing it ([#65](https://github.com/Tooobs/tooob/issues/65))
- ✅ The browser suite runs on Playwright, and can't take over the machine ([#82](https://github.com/Tooobs/tooob/issues/82))
- ⬜ The browser suite's flaky tests: the burst stall ([#19](https://github.com/Tooobs/tooob/issues/19)), the slow exit after the suspend test ([#20](https://github.com/Tooobs/tooob/issues/20)), the renegotiation timeout ([#30](https://github.com/Tooobs/tooob/issues/30))

## [Agents](https://github.com/Tooobs/tooob/milestone/5)

AI agents run Hosts for their Owners and visit Sites in a browser like anyone else; tooob is only the transport ([ADR 0019](adr/0019-tooob-is-the-transport-not-the-signalling.md)).

- ⬜ Agent Accounts: an Agent works within a Host a person made, and invites only with First-Visit Approval ([#63](https://github.com/Tooobs/tooob/issues/63))
- ⬜ A local MCP server lets an agent read, block and revoke ([#9](https://github.com/Tooobs/tooob/issues/9))
- ⬜ An agent's requests become Pending Requests the Owner approves ([#10](https://github.com/Tooobs/tooob/issues/10))
- ⬜ A guide: an agent shares a dashboard ([#64](https://github.com/Tooobs/tooob/issues/64))

## [Relay finder](https://github.com/Tooobs/tooob/milestone/6)

Owners find a Relay to use. Choosing stays the Owner's ([ADR 0015](adr/0015-no-default-relay-or-bootstrap.md), [ADR 0020](adr/0020-a-relay-serves-its-bootstrap.md)).

- ⬜ A public list, or a trusted site with each Relay's stats and terms ([#72](https://github.com/Tooobs/tooob/issues/72))
- ⬜ A Relay audit: what each listed Relay serves, checked against tooob's published Releases ([#74](https://github.com/Tooobs/tooob/issues/74))
- ⬜ Research: what a Relay costs to run, by provider and by what it carries ([#73](https://github.com/Tooobs/tooob/issues/73))

## [Normal people](https://github.com/Tooobs/tooob/milestone/2)

tooob for people who don't open a terminal.

- ⬜ The macOS `tooob` is signed and notarized ([#52](https://github.com/Tooobs/tooob/issues/52)), and Releases include Intel Macs ([#51](https://github.com/Tooobs/tooob/issues/51))
- ⬜ The Host runs on Windows ([#28](https://github.com/Tooobs/tooob/issues/28))
- ⬜ `tooob` upgrades itself ([#54](https://github.com/Tooobs/tooob/issues/54))
- ⬜ A Bootstrap speaks the current and the previous Protocol Version ([#53](https://github.com/Tooobs/tooob/issues/53))
- ⬜ A TUI in place of menus and pickers ([#35](https://github.com/Tooobs/tooob/issues/35))
- ⬜ The Owner chooses whether a Shared Folder shows hidden files ([#29](https://github.com/Tooobs/tooob/issues/29))
- ⬜ A desktop tray app, with guided set-up ([ADR 0022](adr/0022-tooob-asks-nothing.md)); remote MCP with OAuth (no tickets yet)
- ⬜ Idea: the `tooob` binary is a Relay too ([#76](https://github.com/Tooobs/tooob/issues/76))

## [v2](https://github.com/Tooobs/tooob/milestone/3)

- ⬜ A native visitor app or extension, with iroh's own hole punching; friendly names; device linking (no tickets yet)
- ⬜ Idea: browsers reach a Host with no Relay at all ([#75](https://github.com/Tooobs/tooob/issues/75))
- ⬜ Several Hosts serving one Site, so Visitors reach whichever is up ([#17](https://github.com/Tooobs/tooob/issues/17))
- ⬜ Whether search engines and LLMs can index a Site, and how ([#16](https://github.com/Tooobs/tooob/issues/16))
- ⬜ Idea: native phone apps, so a phone can be a Host and a Site goes phone to phone ([#81](https://github.com/Tooobs/tooob/issues/81))

## [Sync](https://github.com/Tooobs/tooob/milestone/4)

A folder kept in step across the Owner's devices and the people they invite. Its scope is still to be settled; no tickets yet.
