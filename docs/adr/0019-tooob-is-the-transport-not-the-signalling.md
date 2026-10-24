# tooob is the transport, never the signalling

tooob carries a Site from where it runs to the people and Agents invited to it, and stops there: it doesn't tell anyone that a link exists, that a device is waiting for approval, or that something changed. Invite Links travel over whatever channel people and Agents already use (a chat, an email, a robofinger note), and whatever does that signalling reads tooob's state from its log and its `--json` output. We rejected integrating with robofinger, or any such tool, and a notification hook: each would tie tooob to one way of coordinating among many, when the value is being the transport any of them can use.

## Consequences

- An Owner learns about a device waiting for approval from `tooob visitors`, the Host's log, or a tool of their choosing that reads them; push notifications for Normal people would come from such a tool, or reopen this ADR.
