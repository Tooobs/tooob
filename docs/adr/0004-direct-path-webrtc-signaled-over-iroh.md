# Browsers get a Direct Path over WebRTC, negotiated over iroh

The Bootstrap opens a WebRTC data channel to the Host, with offer, answer and ICE candidates all sent over the already-authenticated iroh connection. The DTLS fingerprint arrives over iroh, which binds the Direct Path to the Host Key, and the iroh Relay is the fallback in place of a TURN server. Without it, every browser byte would cost Relay bandwidth: the economics would not work, and Relayed Paths are slower for distant Visitors. We first planned relay-only browsers in v1 and reversed that once bandwidth economics turned out to be the main weakness against Cloudflare and Tailscale.

## Consequences

- No separate signaling or TURN servers to run; a STUN server is still needed.
- Two transports to keep working; requests switch to the Direct Path when it comes up and pause back to the Relayed Path when it drops.
