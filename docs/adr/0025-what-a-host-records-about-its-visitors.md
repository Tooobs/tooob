---
status: amended by ADR 0026
---

# What a Host records about its Visitors

A Host keeps a log of its own, in its state folder and readable only by the Owner's account, and nothing in it leaves the Host (ADR 0002). It records Visits (who got in, by Label and short Visitor Key, first visit or return, on which path, and when they left, with the bytes sent and received), switches between the Relayed and Direct Paths, refusals (an unknown or Blocked device, an expired or used Invite, a device waiting for approval), the Owner's changes (Invites, Blocks, approvals, and where each came from), and the Host's own starts, stops and warnings. It never records which pages or files a Visitor opened, or their IP addresses. Per-Visitor byte totals are kept with the Host's state so `tooob visitors` can show them across restarts.

tooob is semi-private: an Owner shares with people they know, and should be able to see that someone came and roughly how much they took, as a router's log shows. Seeing what each person read is surveillance of friends and family, so it isn't recorded, not even as an opt-in. A Site that wants its own analytics can keep them; the Host doesn't. Addresses are left out because the Owner doesn't need them to know who visited (the Label says), and a log of a Visitor's whereabouts is worth protecting. The log is bounded (about 1 MB, plus one older file) so a Host left running for years never fills a disk.

## Consequences

- A Host writes this log itself, with timestamps, however it was started; the service manager's capture of its output only catches a crash. `tooob logs NAME` reads it for any Host (`-n`, `-f`, `--json`), and a Host run in a terminal prints the same lines.
- A Visit is counted per Visitor, not per connection: reloads, extra tabs and network switches within a minute are one Visit.
