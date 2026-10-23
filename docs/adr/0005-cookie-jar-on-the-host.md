# Sites' cookies live in a per-Visitor jar on the Host

The Host keeps a cookie jar for each Visitor Key, adds cookies to requests it proxies, and strips `Set-Cookie` from responses. Browsers can't receive cookies from service-worker responses, so they had to live somewhere. We chose the Host over a jar in the Bootstrap because `HttpOnly` session cookies then never reach the browser at all.

## Consequences

- Cookies set by a Site's own JavaScript don't reach the Host, and server-set cookies aren't readable from JavaScript; Sites that rely on either need work.
- The jars must persist with the Allowlist, or a Host restart logs every Visitor out.
