# Build on iroh instead of our own transport

Hosts, Visitors and Relays speak iroh 1.x (QUIC, keys as addresses, hole punching, WebSocket relays). NAT traversal is the hardest part of the product, and iroh already does it well, is MIT/Apache, and promises wire stability across v1. We considered cutting OpenCola's relay down to a DERP-like core, or writing a new minimal relay, and rejected both: each would have meant writing hole punching ourselves.

## Consequences

- iroh in the browser is relay-only (pages can't open raw UDP), which is why browsers need a separate Direct Path (ADR 0004).
- A WebRTC custom transport inside iroh would eventually be cleaner than our app-level Direct Path, but that API is unstable today.
