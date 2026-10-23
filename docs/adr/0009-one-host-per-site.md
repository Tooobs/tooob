# One Host per Site

Each Site an Owner shares is its own Host, with its own Host Key, Key Address, Allowlist and Invites, even when several Sites run on the same home box under one tooob process. Per-Site access then falls out naturally (Mom gets Jellyfin, not Home Assistant), and each Site gets its own browser origin. We rejected one Host serving several Sites with per-Site grants inside one Allowlist: more model for the common case, and the Sites would share an origin. Hard to reverse: Invite Links and bookmarks carry per-Site Key Addresses.
