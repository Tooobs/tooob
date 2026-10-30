# tooob

A semi-private web: people and their Agents run Sites and let the people and Agents they invite reach them in an ordinary browser, through Relays that can't read the traffic.

## People

**Owner**:
The person who runs a Host and decides who can reach its Site. Always a person: an Agent acts for an Owner, never as one.
_Avoid_: admin, operator, user, host (for the person)

**Visitor**:
One browser on one device that has used an Invite for a Host (a person's, or an Agent's headless one), identified by its Visitor Key: allowed in, awaiting approval, or blocked. A person with a phone and a laptop is two Visitors; so are two Agents' browser profiles on one box.
_Avoid_: guest, friend, user, client

**Standing**:
Where a browser stands with a Host: allowed in, awaiting approval, or denied (never invited, or blocked). The Host tells a browser its Standing without saying which kind of denied.
_Avoid_: status, state, access level

**Owner Device**:
A Visitor that belongs to the Owner and may manage the Host. Created by First-Run Pairing, a Pairing Invite, or an Owner Invite the Owner approves.
_Avoid_: admin device, owner key, controller

**Label**:
The name an Owner gives a Visitor ("mom", "Dave's phone"). Several Visitors can share a Label when they belong to the same person.
_Avoid_: username, nickname, alias

**Relay Operator**:
Whoever runs a Relay, and so serves its Bootstrap: the Owner themselves, or someone the Owner chooses to trust with their traffic's metadata and their Visitors' browsers.
_Avoid_: operator (unqualified), provider, admin, Bootstrap Operator

**Agent**:
An AI program acting for a person: running a Host for its Owner, managing one, or reaching Sites as a Visitor, through a browser like anyone else.
_Avoid_: bot, AI, assistant (unqualified)

**Agent Account**:
A local account on the home box that a Host's settings name as its Agent's. From it, the Agent can only do what an Agent may: read, Block, revoke, and make Invites that need First-Visit Approval. A Host an Agent makes in its own account is that Agent's to run as it likes; one made in a person's account, naming an Agent Account, is the person's.
_Avoid_: service account, bot user, agent user

## Things

**Host**:
One Site made reachable through tooob, with its own Host Key, Allowlist and Invites. Each Host is its own process on the home box, so ending one leaves the others running.
_Avoid_: server, node, daemon, home box, a tooob

**Host Name**:
The short name the Owner knows a Host by on the home box (`photos`, `jellyfin`), and names it by in every command. Only the Owner sees it: Visitors reach a Host by its Host Key, never by its Host Name.
_Avoid_: hostname, alias, id, Friendly Name (that's a public address)

**Site**:
What a Host makes reachable: a web app on the home box, such as Home Assistant or Jellyfin, or a Shared Folder.
_Avoid_: app, service, origin, backend

**Shared Folder**:
A Site the Host serves itself, read-only, from a folder or a single file on the home box, with no separate web app.
_Avoid_: share (as a noun), drive, bucket

**Host Settings**:
What a Host serves and which Relay it uses, in a file the Owner edits by hand (`tooob show` prints where). A Host runs as its settings say, however it's started. The Owner always names the Relay, and its Bootstrap comes with it: there are no defaults.
_Avoid_: config (unqualified), options, preferences

**Host State**:
What a Host keeps for itself and only tooob writes: its Host Key, Allowlist, Invites and cookie jars, in its state folder. Whether the Host is running or stopped isn't part of it.
_Avoid_: data dir, storage

**Autostart**:
Whether the operating system starts a Host by itself: at login (or at boot on a headless Linux box), and again after a crash. `tooob enable` turns it on and `tooob disable` turns it off; `tooob ls` shows it. It says nothing about whether the Host is running now.
_Avoid_: enabled (as what `ls` shows), service, daemon

**Host Key**:
The key that is both a Host's identity and its address, written in z-base-32 (52 characters, short enough to be a hostname's label). Losing it means a new address and re-inviting every Visitor.
_Avoid_: endpoint id, node id, host id

**Visitor Key**:
The key a Visitor's browser generates and keeps; the Host recognizes the Visitor by it, and a new key means a new Visitor.
_Avoid_: session, token, cookie

**Key Address**:
The URL form of a Host Key: the Host Key as a subdomain of its Relay's domain (`<host key>.<relay domain>`). Each is an origin of its own in the browser, walled off from every other Host's. Each of the Site's pages is at its own path on the Key Address, which opens that page directly. Free, unreadable, and as lasting as the Relay. A Host with no Relay, or with a Relay at an IP address, has none, so it can't make Invite Links.
_Avoid_: subdomain, link

**Friendly Name**:
A human-readable address for a Host, in place of its Host Key (`john.<relay domain>`).
_Avoid_: domain, vanity URL, handle, Host Name (that's private to the Owner)

## Access

**Invite**:
A single-use, expiring permission for one device to become a Visitor, carried in an Invite Link.
_Avoid_: share, token, pass

**Owner Invite**:
An Invite whose device becomes an Owner Device once the Owner approves it, except at First-Run Pairing.
_Avoid_: admin invite

**First-Run Pairing**:
Using the first Owner Invite of a Host that has never had an Owner Device: the one time an Owner Invite needs no approval.
_Avoid_: setup, onboarding, claiming

**Pairing Invite**:
A short-lived Owner Invite made on the home box itself, which needs no approval: physical access counts as ownership. It's how an Owner recovers when every Owner Device is lost or blocked.
_Avoid_: recovery code, reset link

**First-Visit Approval**:
An option on an Invite: the device that uses it waits for the Owner's approval before it's let in.
_Avoid_: manual approval, moderation

**Invite Link**:
The URL that carries an Invite: its Host's Key Address, with the Invite after the `#`. Opening it is how a device becomes a Visitor.
_Avoid_: share link, magic link

**Allowlist**:
The Visitors a Host currently lets in. It lives only on the Host.
_Avoid_: whitelist, ACL, friends list, access list

**Block**:
Stopping one Visitor from getting in. The Host keeps the blocked Visitor on record so no Invite can undo it; only an unblock can. Blocking everyone with a Label is a convenience on top, not a different thing.
_Avoid_: ban, kick, revoke (as the general term)

**Upstream Deny**:
A Block the Host also asks its Relay to enforce, so the blocked Visitor's traffic is dropped before it reaches the home box. A Relay can only deny, never admit.
_Avoid_: relay block, firewall rule

**Pending Request**:
A change that would let someone in without the Owner seeing them first (an unblock, an Invite without First-Visit Approval), proposed by an Agent and waiting for the Owner's approval.
_Avoid_: suggestion, draft, proposal

## Network

**Relay**:
A server that forwards encrypted traffic between keys without being able to read it, and serves the Bootstrap for the Hosts that use it.
_Avoid_: server, proxy, TURN server, tunnel

**Relayed Path**:
Traffic between a Visitor and a Host that goes through a Relay.
_Avoid_: tunnel, fallback (as a noun)

**Direct Path**:
Traffic between a Visitor's browser and a Host that goes straight between them, with no Relay in between.
_Avoid_: p2p connection, peer-to-peer link

**Bootstrap**:
The static page a Visitor's browser loads first, from its Host's Relay, which then connects to the Host. Visitors trust its Relay Operator.
_Avoid_: loader, launcher, web client

**Protocol Version**:
Which version of tooob's protocol a Host or a Bootstrap speaks (`tooob/0`). A Host and a Bootstrap work together when they speak the same Protocol Version, whatever their Releases.
_Avoid_: wire version, API version, ALPN

**Bootstrap Mismatch**:
A Relay serving a Bootstrap its Host's tooob can't work with: one that speaks another Protocol Version, or no Bootstrap at all. A Host won't start on one; a running Host that finds one keeps serving, warns, and `tooob ls` and `tooob show` say so. A Relay that can't be reached isn't one: what it serves is unknown until it answers.
_Avoid_: version mismatch, incompatible Relay

## Releases

**Release**:
A numbered version of tooob (`0.1.0`): the `tooob` command for each platform, the Bootstrap Bundle and the Host's container image, all made from one tag.
_Avoid_: build, drop, version (unqualified)

**Bootstrap Bundle**:
The files a Bootstrap is made of, as one Release publishes them, identified by the hash of its manifest, which lists every other file's hash. A Relay serves a Bootstrap Bundle, and anyone can check which.
_Avoid_: www, dist, the build
