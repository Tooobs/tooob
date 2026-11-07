# tooob

A semi-private web: people and their Agents run Sites and let the people and Agents they invite reach them in an ordinary browser, through Relays that can't read the traffic.

## People

**Owner**:
The person who runs a Host and decides who can reach its Site. Always a person: an Agent acts for an Owner, never as one.
_Avoid_: admin, operator, user, host (for the person)

**Visitor**:
One browser on one device that has used an Invite for a Host (a person's, or an Agent's headless one), identified by its Visitor Key: allowed in, awaiting approval, or blocked. A person with a phone and a laptop is two Visitors; so are two Agents' browser profiles on one box.
_Avoid_: guest, friend, user, client

**Visit**:
A Visitor's time on a Site, from their first connection until a minute after their last; reloads, more tabs and network switches in between are the same Visit.
_Avoid_: session, connection (as what the Owner sees)

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
A Site the Host serves itself, from a folder or a single file on the home box, with no separate web app. Read-only, unless its Owner turns on uploads: then its Visitors can add files to it, outside its hidden folders, but never change or remove any. A home folder never takes uploads.
_Avoid_: share (as a noun), drive, bucket

**Synced Folder**:
A folder kept in step between two of the Owner's boxes, each with its own copy on disk and its own Host: what changes on one, added, edited, renamed or removed, reaches the other once both are online. A change made on both while apart keeps both versions. Either Host can also serve its copy to Visitors as a Shared Folder.
_Avoid_: sync (as a noun), mirror, replica, share

**Host Settings**:
What a Host serves and which Relay it uses, in a file the Owner edits by hand (`tooob show` prints where). A Host runs as its settings say, however it's started. The Owner always names the Relay, and its Bootstrap comes with it: there are no defaults.
_Avoid_: config (unqualified), options, preferences

**Host State**:
What a Host keeps for itself and only tooob writes: its Host Key, Allowlist, Invites, cookie jars and what each Visitor has used (bytes sent and received, and when they were last seen), in its state folder. Whether the Host is running or stopped isn't part of it.
_Avoid_: data dir, storage

**Backup**:
One Host, sealed in a single file under a passphrase, to bring it back on another box (`tooob backup NAME FILE`, then `tooob restore NAME FILE`): its Host Key, Allowlist, Invites and Blocks, and its Host Settings, so it keeps its Key Address and its Visitors. Cookie jars and what each Visitor has used are left out, since they're rebuilt. Without the passphrase the file is unreadable, and restoring never replaces a Host already there. Restoring is the only time a Host's Site can move: `--serves PATH` says where it is on the new box (ADR 0029).
_Avoid_: export, import, dump, snapshot

**Autostart**:
Whether the operating system starts a Host by itself: at login (or at boot on a headless Linux box), and again after a crash. `tooob run NAME -d` turns it on; `tooob stop` turns it off and keeps it off until `tooob start` turns it on again; `tooob ls` shows yes, stopped or no. It says nothing about whether the Host is running now.
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
A single-use, expiring permission for one browser to become a Visitor, carried in an Invite Link. Another browser on the same device, or a private window, is another Visitor and needs an Invite of its own.
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
An option on an Invite: the device that uses it waits for the Owner's approval before it's let in. The Owner approves it only once its Device Code matches the one its person reads them.
_Avoid_: manual approval, moderation

**Device Code**:
Six digits, worked out from a Visitor Key, that a device shows while it waits for the Owner's approval (First-Visit Approval, or an Owner Invite) and the Owner sees beside it, so the Owner approves the device they meant to. Shown only while waiting.
_Avoid_: PIN, verification code, pairing code

**Sync Invite**:
A single-use, short-lived permission for another of the Owner's boxes to join a Synced Folder, made on a box already in it. Using it starts a pairing that both boxes must confirm by their Sync Code.
_Avoid_: sync link, join code

**Sync Code**:
Six digits, worked out from both boxes' Host Keys and the Sync Invite, that both boxes show while they pair. The Owner confirms on each box that the two match, so a box that got hold of the Sync Invite can't pair in between unseen. Shown only while pairing.
_Avoid_: Device Code (that's one device's, for the Owner's approval), PIN, pairing code

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
A server that forwards encrypted traffic between keys without being able to read it, and serves the Bootstrap for the Hosts that use it. Once its Relay Operator has enrolled any key, it carries only traffic to or from an Enrolled Key.
_Avoid_: server, proxy, TURN server, tunnel

**Enrolled Key**:
A key a Relay carries traffic for, enrolled by its Relay Operator: in practice a Host Key. The Relay forwards only what's sent to or from an Enrolled Key, so a Visitor reaches an Enrolled Host with nothing to enroll, while strangers can't use the Relay between themselves. A Relay with no Enrolled Keys is open to everyone. Enrolling is the Relay Operator's call about the Relay's bandwidth; who may see a Site stays the Host's Allowlist.
_Avoid_: allowlisted key (the Allowlist is the Host's), registered host, member

**Enrollment Code**:
A one-time code a Relay Operator makes so an Owner can enroll a Host on that Relay. It names its Relay, and the Host redeems it as it first connects, proving its Host Key. One Host per code and unused after 7 days it expires, unless the Relay Operator made it for more Hosts or longer.
_Avoid_: invite (an Invite is a Visitor's), token, pass, voucher

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

**Upgrade**:
Replacing a box's `tooob` command with another Release, done only when its Owner asks. A running Host keeps the Release it started with until it restarts.
_Avoid_: update, self-update, auto-update

**Bootstrap Bundle**:
The files a Bootstrap is made of, as one Release publishes them, identified by the hash of its manifest, which lists every other file's hash. A Relay serves a Bootstrap Bundle, and anyone can check which.
_Avoid_: www, dist, the build
