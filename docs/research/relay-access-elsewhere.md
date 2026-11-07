# How other relays decide who may use them

Checked 2026-11-07, for [#98](https://github.com/Tooobs/tooob/issues/98). **The question:** how do projects that run a relay decide who may use it? For each: what identifies a permitted client; enrolment and revocation; whether it's checked per connection, packet or destination; how a client without credentials (a browser, an unknown peer) gets in; and what a public address means. Code is cited at a tag; anything unconfirmed is in [Unverified](#unverified).

## iroh-relay 1.3.0 (what tooob runs)

From the crate source, the same as the [`v1.3.0` tag](https://github.com/n0-computer/iroh/tree/v1.3.0).

- **What it offers.** `access` is `everyone` (the default), `allowlist` / `denylist` (endpoint IDs), `shared_token` (exact match against a list, read from `Authorization: Bearer` or `?token=`), or `http` ([`main.rs` L160-197](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/main.rs#L160-L197)). The endpoint ID is "proven by the relay handshake" before the hook runs ([`server.rs` L189-L240](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/server.rs#L189-L240)).
- **A correction to #98.** The `http` mode in the binary POSTs only `X-Iroh-Endpoint-Id`, not the client's headers or URL ([`main.rs` L312-336](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/main.rs#L312-L336)). The library trait `AccessControl::on_connect` sees the whole `ClientRequest` (headers, URI, `auth_token()`), so checking `Origin` or a signed token means embedding the library.
- **When it's checked.** "Called once per incoming connection" ([`server.rs` L285-305](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/server.rs#L285-L305)). There is no per-packet or per-destination hook: `Clients::send_packet` (`pub(super)`) looks up `dst` and forwards ([`clients.rs` L200-L215](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/server/clients.rs#L200-L215)). An embedder can drop an endpoint with `clients().disconnect(...)` ([`http_server.rs` L949-955](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/server/http_server.rs#L949-L955)): revocation, not routing.
- **Browsers.** The client's `auth_token` is sent "as a `?token=` query parameter under Wasm" ([`client.rs` L155-L245](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/client.rs#L155-L245)); `RelayConfig::with_auth_token` sets it per relay.
- **Limits.** Only a per-connection receive rate; accept-rate limits are "not currently implemented" ([`server.rs` L489-L510](https://github.com/n0-computer/iroh/blob/v1.3.0/iroh-relay/src/server.rs#L489-L510)).
- **n0's managed relays** ([blog, 2026-07-30](https://iroh.computer/blog/authenticated-relays)). Authenticated by default: each endpoint sends (`Authorization: Bearer`) "a signed capability token" whose issuer is the project's API key, subject the endpoint's public key, grant relay use only, with an expiry. The `iroh_services` preset mints it locally from the API secret. Deleting or rotating the key drops its connections. Relays from before June 2026 stay open unless switched on.

## Tailscale DERP

- DERP addresses peers "using WireGuard public keys"; a `SendPacket` frame is "32B dest pub key + packet bytes" ([`derp/README.md`, `derp.go` L75, v1.104.5](https://github.com/tailscale/tailscale/blob/v1.104.5/derp/derp.go#L75)).
- **`--verify-clients`.** It asks the local `tailscaled` `WhoIsNodeKey`, so it admits only node keys that `tailscaled` knows as peers in its tailnet.
- **`--verify-client-url`.** It POSTs `DERPAdmitClientRequest{NodePublic, Source IP}` and admits on `{"Allow": true}`. The request times out after 5 s, and `--verify-client-url-fail-open` defaults to **true** ([`derper.go` L83-85](https://github.com/tailscale/tailscale/blob/v1.104.5/cmd/derper/derper.go#L83-L85); [`derpserver.go` L1761-1823](https://github.com/tailscale/tailscale/blob/v1.104.5/derp/derpserver/derpserver.go#L1761); [`tailcfg/derpmap.go` L267](https://github.com/tailscale/tailscale/blob/v1.104.5/tailcfg/derpmap.go#L267)). Mesh peers bypass both checks.
- **When it's checked.** Once, at connect. `handleFrameSendPacket` forwards to any connected key with no policy check ([L1470](https://github.com/tailscale/tailscale/blob/v1.104.5/derp/derpserver/derpserver.go#L1470)).
- **Enrolment and revocation** are tailnet membership; a removed node is refused on its next connect. Every party is a tailnet node, so there are no unknown peers.
- **Public address.** "Anyone that knows the IP address of your DERP server could add it to their DERP map", and `--verify-clients` is the documented guard ([KB 1118](https://tailscale.com/kb/1118/custom-derp-servers), validated Oct 7, 2026).

## Headscale's embedded DERP

`derp.server.verify_clients: true` is the default ([`config-example.yaml`, v0.29.7](https://github.com/juanfont/headscale/blob/v0.29.7/config-example.yaml#L106-L107)). It sets the embedded server's verify URL to an in-process `/verify`, failing closed ([`derp_server.go` L60-62](https://github.com/juanfont/headscale/blob/v0.29.7/hscontrol/derp/server/derp_server.go#L60-L62)). That handler allows a node key if any registered node has it ([`handlers.go` L136-167](https://github.com/juanfont/headscale/blob/v0.29.7/hscontrol/handlers.go#L136-L167)). An external `derper` can use `--verify-client-url https://<headscale>/verify` ([docs/ref/derp.md](https://github.com/juanfont/headscale/blob/v0.29.7/docs/ref/derp.md)). Enrolment is node registration, revocation deleting the node; checked per connection.

## Syncthing relays (strelaysrv)

- **The public pool.** By default a relay joins relays.syncthing.net, open to any Syncthing user; `-pools=""` keeps it private. Rate limits: `-global-rate` and `-per-session-rate` in bytes/s ([strelaysrv docs](https://docs.syncthing.net/users/strelaysrv.html), written for v2.1.0).
- **Who a client is.** Its Device ID, taken from its TLS certificate in protocol mode ([relay protocol v1](https://docs.syncthing.net/specs/relay-v1.html)).
- **`-token`** ("Added in version 1.22.1") turns pool announcement off. Clients put the token in the relay URI (`relay://host:port/?id=<relay id>&token=...`).
- **The closest analogue to #98.** In the source, the token is checked **only on `JoinRelayRequest`**, the permanent mode a device enters to be reachable. A `ConnectRequest` to a joined device is not token-checked. It only needs the target to be joined ([`listener.go` L126-133, L165-185, v2.1.7](https://github.com/syncthing/syncthing/blob/v2.1.7/cmd/strelaysrv/listener.go#L126-L185)). So a private relay serves token holders, and **anyone who asks for a token holder by Device ID**. Revoking means changing the token.
- The spec shows `JoinRelayRequest` empty; the source has a `Token` ([`packets.go` L37-39](https://github.com/syncthing/syncthing/blob/v2.1.7/lib/relay/protocol/packets.go#L37-L39)).

## TURN and coturn

- **Long-term credentials** ([RFC 8489 §9.2](https://www.rfc-editor.org/rfc/rfc8489.html#section-9.2), Feb 2020). A username and password "provisioned for a user", lasting until the user leaves; a server nonce gives limited replay protection.
- **What TURN authenticates** ([RFC 8656](https://www.rfc-editor.org/rfc/rfc8656.html), Feb 2020): requests (Allocate, Refresh, CreatePermission, ChannelBind), but "indications in TURN are never authenticated". Data is bound to the allocation's 5-tuple instead.
- **The per-destination check:** a **permission** per peer IP, installed by an authenticated request. It expires after 5 minutes, and other traffic "is silently discarded" (§3.3). Quotas "SHOULD" be per username, not per transport address.
- **TURN REST** ([draft-uberti-behave-turn-rest-00](https://datatracker.ietf.org/doc/html/draft-uberti-behave-turn-rest-00), Uberti, July 2013; individual draft, expired 2014). Built because "TURN credentials must be specified in Javascript".
  - **The flow.** The app's backend answers `GET ?service=turn&username=...` with `{username, password, ttl, uris}`: username `<expiry>:<user id>`, password `base64(HMAC(secret, username))`, suggested TTL 86,400 s.
  - **The check.** The TURN server rejects an expired timestamp, then verifies MESSAGE-INTEGRITY with the password derived from the username. Only the secret is shared.
  - **Revocation.** None per credential: wait for expiry or blacklist a username. Rotate the secret, accepting two during rollover.
  - **Gating the backend:** IP, Origin, User-Agent, a login cookie or an API key.
- **coturn 4.18.0.** `use-auth-secret` with `static-auth-secret`, or with secrets in the `turn_secret` database table ("dynamic"). The username is `"timestamp:userid"` and the password is `base64(hmac(secret key, usercombo))` ([`turnserver.conf` L313-356](https://github.com/coturn/coturn/blob/4.18.0/examples/etc/turnserver.conf#L313-L356)). It also has `user-quota`, `total-quota`, `max-bps`, `bps-capacity`, and `denied-peer-ip` / `allowed-peer-ip` (destination filtering). `oauth` implements [RFC 7635](https://www.rfc-editor.org/rfc/rfc7635.html) (Aug 2015): a self-contained, expiring token encrypted under a key the authorisation server shares with the TURN server.
- **How apps hand credentials to browsers:**
  - **Synapse (Matrix)** mints TURN REST credentials for any logged-in user at `GET /voip/turnServer`: `expiry:user_id`, HMAC-SHA1, `turn_user_lifetime` 1 h and `turn_allow_guests` true by default ([`voip.py` L47-67](https://github.com/element-hq/synapse/blob/v1.164.2/synapse/rest/client/voip.py#L47-L67); [`config/voip.py` L56-59](https://github.com/element-hq/synapse/blob/v1.164.2/synapse/config/voip.py#L56-L59), v1.164.2).
  - **Cloudflare Realtime TURN.** The backend calls `generate-ice-servers` with `{"ttl": ...}` and passes the `iceServers` to the front end. A credential can be revoked by its username ([docs](https://developers.cloudflare.com/realtime/turn/generate-credentials/), updated Sep 25, 2026).
- **Public address.** The secret stays on the backend; the browser only holds a short-lived credential for an identity the backend checked.

## ngrok and Cloudflare Tunnel

The server side is enrolled and visitors are anonymous.

- **ngrok.** An agent authtoken belongs to a User or Service User; its only ACL rule is `bind` (what it may serve); deleting the credential revokes it ([credentials API](https://ngrok.com/docs/api-reference/credentials/create), [Service Users](https://ngrok.com/docs/iam/bot-users/)).
- **Cloudflare Tunnel.** "Anyone with access to the token will be able to run the tunnel". After a compromise, refresh the token and delete the existing connections ([docs](https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/configure-tunnels/remote-tunnel-permissions/), updated Jun 23, 2026).
- **Visitors** need nothing. The public hostname only reaches that account's enrolled tunnel.

## ZeroTier

A node's address derives from its public key. Roots relay packets and answer identity lookups; "planetary roots know about all nodes" and "don't join or control virtual networks". Membership is a controller-signed certificate that peers check, not roots ([protocol](https://docs.zerotier.com/protocol/)), so the public roots appear to serve any node. Moons (private supplementary roots) are now "not recommended", and the docs give them no access control ([private roots](https://docs.zerotier.com/roots/)).

## Nebula

Every host has a CA-signed certificate ([README, v1.12.0](https://github.com/slackhq/nebula/blob/v1.12.0/README.md)); lighthouses and relays are ordinary hosts, so only talk to CA-signed peers. Nebula has no anonymous clients.

- **Relaying is opt-in on both sides.** The relay sets `am_relay`, and a host names its relays in `relay.relays`. A relay "will only forward traffic for hosts which specify it as a relay" ([relay config](https://nebula.defined.net/docs/config/relay/), since v1.6.0).
- **Each relay request is checked.** Requests come over an established tunnel; the relay discards a request "where relayFrom is not the sending peer" and only relays to peers it is directly connected to ([`relay_manager.go` ~L527-547](https://github.com/slackhq/nebula/blob/v1.12.0/relay_manager.go#L527-L547)).
- **Enrolment** is a signed certificate; **revocation** its expiry or a blocklist (see Unverified).

## Others, briefly

- **libp2p circuit relay v2** ([spec r3, 2023-02-28](https://github.com/libp2p/specs/blob/master/relay/circuit-v2.md)). The *target* `RESERVE`s (expiring, with a relay-signed voucher); an initiator with **no** reservation `CONNECT`s to it by peer ID. Circuits have optional `duration` and `data` limits, and ACLs may answer `PERMISSION_DENIED`. Syncthing's shape again.

## Unverified

- Whether Tailscale's own DERP fleet verifies clients; no primary page says.
- That ZeroTier's planetary roots relay for nodes on no network: inferred, not stated.
- Nebula's `pki.blocklist` on v1.12.0; the `relay_manager.go` line range is approximate.
- n0's token format, and whether self-hosted iroh-relay can verify it (1.3.0's `shared_token` is an exact match).
- Whether coturn takes several `static-auth-secret` lines for a rollover (its database table can hold several).

## Patterns that fit tooob's constraints

- **Listener holds the right, callers ride on it** (Syncthing `-token`, libp2p reservations). Enrol Hosts; admit anyone whose only destinations are Enrolled Hosts. iroh-relay has no forwarding hook for the second half: a fork, wrapper or own relay.
- **TURN REST, with the Host as the backend.** An Enrolled Host Key signs a short-lived pass (`expiry:id` + signature) carried in the Invite Link; the Bootstrap sends it as `?token=`, as iroh's Wasm client already does; an embedded `AccessControl` checks it against the enrolment list, with no shared state.
- **Capability token bound to the subject key** (n0's managed relays). For Hosts: a Relay Operator's signature over the Host Key, not a list or a bearer secret.
- **Admission hook to the enrolment list** (DERP `--verify-client-url`, Headscale `/verify`). The stock `http` mode for Host Keys; fail closed, unlike derper's default.
- **Revoke by expiry plus a disconnect** (TURN REST, Cloudflare Tunnel). Short passes; `clients().disconnect` for a revoked Host.
- **Bound the damage per credential** (coturn's per-user quotas, Syncthing's per-session rate, libp2p circuit limits). Limit Visitors per pass or per Host, not per connection.
