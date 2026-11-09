# A Relay's sign-up is its Relay Operator's own service, enrolling through a token API

Settled by the Owner on 2026-11-09 (#139), building on ADR 0030. Enrollment Codes (ADR 0030) need the Relay Operator to make and hand over a code for every Owner. A Relay Operator who wants Owners to enroll themselves, after a payment, a Patreon tier, a Discord role or an invite list, needs a page that does it, like `fly auth login`: the CLI gives a link, the browser does the rest.

That page is the Relay Operator's own service, a Sign-up Page, never part of tooob. tooob-relay adds only what such a service needs. The Relay Operator sets the page's address (`--sign-up-url`), `/__tooob/relay.json` reports it as `sign_up`, and on a closed Relay, wherever tooob says a Host isn't carried yet (`add`, `share`, `run` and its log, `ls`), it adds that link with `?key=<Host Key>` and nothing else about the Host or its Site. The page checks whatever it likes, then enrolls the key through an API on the Relay's public address: `PUT /__tooob/api/enrolled/<key>` (with a label) and `DELETE /__tooob/api/enrolled/<key>`, each with an Operator Token as `Authorization: Bearer`. The Relay Operator makes, lists and revokes Operator Tokens with `tooob-relay token`, stored only hashed; failed attempts are limited (10 a minute from an address).

`tooob add` prints the link and returns, never waiting (ADR 0022: tooob asks nothing). A Host that isn't carried, on a Relay with a Sign-up Page, checks its enrollment every 30 seconds instead of every 10 minutes, so it's carried soon after the sign-up.

## Considered and rejected

- **A sign-up page inside tooob-relay, with plug-ins for Stripe, Patreon and Discord.** tooob would own every provider's payment and membership code, secrets and webhooks, and each Relay Operator's choice of how to qualify would wait on tooob shipping it.
- **The page handing back an Enrollment Code to paste in.** The link already carries the Host Key, so the page can enroll it directly; an Owner who signs up before having a Host makes one first, since `add` returns at once.
- **`add` waiting until the Host is enrolled.** A wait of minutes, or forever, is close to the question ADR 0022 rules out, and it breaks scripts.
- **Proving the Owner holds the Host Key** (the page asks the Host to sign a challenge). Enrolling a key only helps whoever holds it, so the one attack it stops (enrolling someone's key on an account that then lapses, removing it) gains the attacker nothing; the API's docs warn Sign-up Pages not to remove a key another account still pays for.
- **An end date on each Enrolled Key**, so tooob-relay removes lapsed ones itself. Lapses, grace periods and warnings are the Relay Operator's policy; their service removes the key, and the Owner learns from the "not carried" notice (ADR 0030).

## Consequences

- The Relay's first public endpoint that changes anything. It does nothing without an Operator Token, and the admin page (ADR 0030) stays off the public address.
- The API is a contract outside services depend on: changing it breaks Sign-up Pages tooob doesn't see.
- tooob ships the mechanism and its docs, with a `curl` example of each call, not a Sign-up Page; one for r.tooobs.net would be a decision of its own.
- An Enrolled Key keeps one label; enrolling it again relabels it, so a Sign-up Page that shares keys between accounts tracks that itself.
