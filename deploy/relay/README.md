# Self-hosted iroh-relay

Stock `n0computer/iroh-relay:v1.3.0` with automatic Let's Encrypt certificates. This is the relay the design calls for in place of n0's community relays.

## Deploy (about 10 minutes)

**Easiest:** run `./wizard.sh`. It walks through Hetzner, DNS, the firewall, Docker, upload and certificate, then measures the spike through the new relay. It remembers answers in `.env` (gitignored), so it's safe to re-run. The manual steps below are what it does.

1. **VPS**: any small Linux box with generous included bandwidth, near your first users. A Hetzner or OVH entry plan works. 1 vCPU and 2 GB RAM is plenty. Install Docker.
2. **DNS**: point an A record (and AAAA if the VPS has IPv6), e.g. `relay.<a domain you own>`, at the VPS. Let's Encrypt needs this before the first start.
3. **Firewall**: open TCP 80 and 443, and UDP 7842.
4. Copy this folder to the VPS. Set `hostname` and `contact` (a real email) in `iroh-relay.toml`, then:
   ```sh
   docker compose up -d && docker compose logs -f   # wait for the certificate, then Ctrl-C
   curl https://relay.<your-domain>/healthz
   ```

## Point the spike at it

```sh
cd spike/local-host && ../target/release/host 8000 https://relay.<your-domain>/
# prints:  visit: http://localhost:8088/?relay=https://relay.<your-domain>/#<id>
cd ../check && node check.mjs "<that visit URL>"
```

For a real-world number, run the host on a home connection and open the visit URL from a phone on cellular data. The visit URL needs the bootstrap (`spike/www`) reachable from the phone too. The simplest option is to serve `www/` from the same VPS behind HTTPS (service workers require a secure origin anywhere except localhost).

## Notes

- `access` is unset, so anyone can use this relay. Fine for the spike. Before sharing it publicly, restrict it (allowlist, tokens or an HTTP callout; see `access` in iroh-relay's `src/main.rs`) and set `limits`.
- Watch the VPS's bandwidth graph: every byte a browser visitor loads goes through here.
