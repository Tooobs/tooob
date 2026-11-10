# A Relay on a VPS

The Relay image (`image/`, see [../README.md](../README.md)) run by Docker Compose on any Linux box. 1 vCPU and 2 GB RAM is plenty; pick a plan for its included traffic.

**Easiest:** `./wizard.sh`. It walks through a Hetzner server, DNS, the DNS API token, the firewall, Docker and the first deploy, then closes the Relay by enrolling your first Host (or printing an Enrollment Code for it). It remembers your answers in `.env` (gitignored, mode 0600; it holds the token), so it's safe to re-run.

## By hand

1. **Server**: a Linux VPS you can SSH into, with Docker (`curl -fsSL https://get.docker.com | sh`).
2. **DNS**: A records (and AAAA if it has IPv6) for `relay.example.com` and `*.relay.example.com`, pointing at the server. On Cloudflare, make them "DNS only".
3. **DNS API token**: one that can edit the zone's records. On Cloudflare: My Profile → API Tokens → "Edit zone DNS", limited to that zone.
4. **Firewall**: open TCP 80 and 443.
5. **`.env`** in this folder:
   ```sh
   VPS_IP='203.0.113.7'
   SSH_USER='root'
   RELAY_DOMAIN='relay.example.com'
   DNS_PROVIDER='cloudflare'          # a caddy-dns module that takes just a token
   DNS_API_TOKEN='…'
   ACME_EMAIL='you@example.com'       # Let's Encrypt's contact; not example.com
   ```
6. **`./deploy.sh [RELEASE]`**: builds and starts the Relay on the server (in `~/tooob-relay`) with that Release's Bootstrap, then waits until `https://relay.example.com/` and a Key Address under it serve it. The first build takes a few minutes, and the certificates a minute or two more.

7. **Close it**: it starts open, carrying anyone's traffic. Enroll your first Host (below, under Running it). `deploy.sh` never enrolls anything, so redeploys leave the Enrolled Keys as they are.

Point Hosts at it with `tooob add … --relay https://relay.example.com/`, or `tooob add … --relay-code <code>`.

## Running it

- Upgrade its Bootstrap: `./deploy.sh <newer Release>`. Caddy restarts, and Hosts and Visitors reconnect by themselves.
- Logs: `ssh root@<server> 'cd tooob-relay && docker compose logs -f'`.
- Stop it: `ssh root@<server> 'cd tooob-relay && docker compose down'`.
- Enroll a Host: `ssh root@<server> 'cd tooob-relay && docker compose exec -T relay tooob-relay enroll <host key> --label <name>'`, or make an Enrollment Code for it with `tooob-relay code --label <name>` there. A Relay with no Enrolled Keys is open to everyone; the first one closes it. To close one that's open today, see [../README.md](../README.md#closing-an-open-relay).
- Watch the server's bandwidth: every byte a Visitor loads over the Relayed Path goes through here.

## A test Relay on your own network

To try what needs a closed Relay (Enrolled Keys, Operator Tokens, the enroll API, a Sign-up Page, the admin page) without touching your real one, run the same image on any Linux box at home with [podman](https://podman.io/), over plain http: no domain, no certificate, no open ports.

```sh
./test-relay.sh start 192.168.0.79:8480 http://192.168.0.10:8787/join   # HOST:PORT SIGN_UP_URL [RELEASE | --local]
./test-relay.sh stop
```

`HOST:PORT` is the box's address on your network (pick a port above 1024 for rootless podman, and let your firewall allow it there); the Sign-up Page is any page you name. It starts closed: it enrolls a stand-in key no Host has, so it carries only the Hosts you enroll. Its Enrolled Keys and Operator Tokens are in the podman volume `tooob-test-relay-data`, kept by `stop` and the next `start`; `podman volume rm tooob-test-relay-data` starts afresh.

- Point a Host at it: `tooob add … --relay http://192.168.0.79:8480/`.
- Its `tooob-relay`: `podman exec tooob-test-relay tooob-relay list` (or `enroll`, `token new`, `code`).
- Its admin page: `ssh -L 8081:127.0.0.1:8081 <box>`, then http://localhost:8081/.
- Not here: browser Visits (a service worker needs https), and Enrollment Codes, which always name an https Relay.

The plain http is the image's `RELAY_PLAIN_HTTP` option (`image/plain-http.Caddyfile`), off unless set; `deploy.sh` and the Fly setup never set it.
