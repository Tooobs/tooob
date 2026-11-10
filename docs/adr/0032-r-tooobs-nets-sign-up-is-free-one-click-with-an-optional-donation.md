# r.tooobs.net's sign-up is free, one click, with an optional donation

Settled by the Owner on 2026-11-10, building on ADR 0031, which left a Sign-up Page for r.tooobs.net as a decision of its own. tooob's pitch to Owners is that sharing is quick: no account for a Visitor, and nothing to install but a browser. Once r.tooobs.net is closed (ADR 0030), a new Owner's Host isn't carried until it's enrolled, so the sign-up is the step that pitch stands or falls on.

r.tooobs.net's Sign-up Page is `https://tooobs.net/join`, a route on the tooobs.net Cloudflare Worker (Tooobs/tooobs.net), not part of tooob. Clicking the link a Host prints (`?key=<Host Key>`) is the whole sign-up: once Cloudflare Turnstile passes, the page asks the Relay whether it carries the key already (and if so, says so and changes nothing), else enrolls it through the enroll API with the label `signup <date>`, and says the Host is carried now. Only then, under that, it asks for a donation: a plain link to Ko-fi, no amount, no reminder, never a condition of enrolling. An enrollment is free and never ends; the Relay Operator removes abusive Hosts by hand.

Against scripts, besides Turnstile, the Worker allows 5 enrollments a day from an address (an IPv6 address counted by its /64), counted in Workers KV with a 24-hour expiry; KV isn't exact, and a limit against abuse doesn't need to be. The page stores the Host Key and the date, nothing else, and says so, with a line that Hosts abusing the Relay are removed; there's no terms checkbox. The Worker holds the Turnstile secret and an Operator Token as secrets.

## Considered and rejected

- **Keeping r.tooobs.net open and asking for donations.** An open Relay carries anyone's traffic, and its address is public: donations don't stop abuse or grow with what it costs, and its Relay Operator can't tell whose traffic it carries.
- **A donation or payment before enrolling.** It's a sign-up step, which the pitch rules out; asking after the Owner has what they came for costs them nothing.
- **Payment in the page (Stripe Checkout and webhooks).** Payment code and more secrets for a donation a plain link takes as well.
- **A "Carry my Host" button.** A second click for what Turnstile already decides; a link preview can't pass Turnstile.
- **An email to confirm, or an account.** A step, and personal data to keep.
- **Enrollments that end unless renewed, or kept only for donors.** Something to run on a schedule, and a sign-up again; possible later if use or abuse grows.
- **A Durable Object for an exact count.** More code for exactness the limit doesn't need.

## Consequences

- The page is only useful once r.tooobs.net is closed; closing it, with `--sign-up-url https://tooobs.net/join` and the Owner's own Hosts enrolled first, is the Owner's step.
- The page depends on the enroll API (ADR 0031) as any outside Sign-up Page does, and it's tested against a closed tooob-relay of the Owner's own before r.tooobs.net closes.
- Abuse is handled after the fact, by hand: a Host can be enrolled and used before anyone notices.
- r.tooobs.net's running costs fall on its Relay Operator, offset only by what donations bring.
