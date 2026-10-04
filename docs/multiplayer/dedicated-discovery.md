# Dedicated server discovery and admission

Dedicated discovery uses a saved/direct IPv4 UDP endpoint and does not require
Steam, a central directory or a paid service. Existing Steam lobby browsing and
peer sessions retain their existing protocol. The optional query protocol is
served from the server's gameplay UDP address even when accounts are required;
it exposes only public operational metadata, never authentication bundles,
account names, settings secrets or resource scripts.

Configure a server with `--operations operations.json`:

```json
{
  "name": "Community park",
  "map": "Community park v1",
  "mode": "free-skate",
  "queue_capacity": 128,
  "queue_timeout_ms": 120000,
  "reserved_slots": 2,
  "required_permission": "community.join"
}
```

The fields default to a generic server name/current map/free-skate mode, 128 queue
entries, 120-second queue lifetime, no reserved slots and no extra permission
policy. Queue size is 1–256, lifetime 1–600 seconds, and reserved slots must be
less than capacity. Omit `required_permission` on anonymous development servers;
a configured permission policy rejects anonymous identities. Names and labels
are bounded and reject control characters.

`skate_net::discovery::query(nonce)` constructs a fixed 1200-byte request.
`parse_response(bytes, nonce)` validates the matching response and metadata
bounds. Responses never exceed the request size. The host allows at most one
response per source IP per second with a bounded 128-source rate table. Treat
all discovery data as untrusted and stale until refreshed; it cannot grant
admission or select credentials. Responses include session ID, base-map
fingerprint, name/map/mode, discovery protocol/build, occupied slots/capacity,
queue count, maintenance/account requirement, public content revision/bytes/count
and up to eight resource names (with explicit omitted count). Explicitly public
typed settings are included within the same response bound; an omitted-setting
count identifies values that did not fit. Private and merely replicated settings
are excluded. Metadata is not a
signed identity statement. Account login still verifies configured TLS trust and
hostname before authenticated gameplay.
When a resource world is active, the map label follows that world's resource ID
through rotation; otherwise it uses the configured map label.

New dedicated HELLOs enter a bounded queue before the existing compatibility,
content-readiness, instance and native-authority guards. Existing connections
continue through their established transport. All connections, including content
downloaders, occupy slots. Eligible public joins are FIFO; an account possessing
live `admission.reserved` permission may use withheld reserved slots. Reservation
never comes from an actor ID, display name, event payload or requested class.
Only the verified transport's account handle can confer it. One account cannot
consume multiple reserved admissions using several login sessions. Privileges and active
session validity are rechecked immediately before admission.

The optional permission check runs on a dedicated bounded worker, so it cannot
perform blocking work in the game tick. There are at most 256 policy requests;
waiting policies fail after three seconds. Results address a unique queue ticket,
so a cancelled/expired connection cannot be admitted by a late result. The
built-in policy checks a configured live account permission. It does not run
arbitrary remote hooks or a recipe-supplied executable.

One account can hold one backlog entry beyond currently free public capacity.
Separate logins that fit available public slots remain usable, including local
multiplayer testing. Another connection cannot take over a waiting entry's
place. Anonymous development servers permit at most four backlog entries beyond
free public capacity from one source IP, with no privileged reservations. Anonymous identity remains
unverified: these limits bound development behavior and are not Internet Sybil
protection. Entries expire after their absolute queue lifetime or five seconds
without HELLO refresh. Rejections are retained for up to one minute in a bounded
256-entry registry; reconnect with fresh credentials after policy changes.
Changing capacity promotes waiting eligible entries on the next tick. Maintenance
rejects new and queued joins without bypassing any account whitelist or resource
readiness rules.

The client `Session.join_status` reports `policy`, `queued`, `admitting`,
`rejected`, `expired`, `maintenance` or a restart `warning`, with position,
remaining milliseconds and bounded reason text. Terminal failures stop automatic
HELLO retries; an explicit new join creates a fresh Session. `cancel_join()`
sends GOODBYE and suppresses later HELLOs. Queue status is bound to the target actor attempt and only accepted from the
configured server endpoint, and account-required transports authenticate its
envelope. Content downloading/activation proceeds through the existing resource
client only after admission; a queue position is not permission to load content.
Dedicated compatibility rejection packets also carry the target actor after
their reason byte. They remain terminal even when queued progress arrives late;
an accepted roster takes precedence over stale pre-admission statuses. Cancelled
attempts ignore late rosters. Steam peer rejection packets retain their existing
format and behavior.

The game browser adds saved endpoints, favorites/recent joins and content/progress
presentation over this API. Server metadata never changes Steam discovery, direct
UDP account configuration, local Lua mods or native competition admission guards.

Focused tests:

```sh
cargo test --locked -p skate-net discovery::tests
cargo test --locked -p skate-server --lib operations::tests
cargo test --locked -p skate-server --test udp --test accounts -- --test-threads=1
```

## Game controls

Open **Multiplayer → Dedicated servers**. Enter a numeric IPv4 endpoint and optional local account-profile filename, then **Save**, **Refresh** and **Join**. Arrow keys/mouse select rows; type or Backspace while an endpoint/profile field is selected. Favorites and successful join timestamps persist alongside `settings/player.json` in `dedicated-servers.json`. Set `SKATE3_DEDICATED_SERVERS` to a local pathname to use a separate endpoint book. No password or transport token is saved in that book.

The browser shows at most 64 saved entries, a 30-second freshness window, compatibility, map/mode, occupancy, content size/resource names and bounded advertised public settings. Offline/stale metadata cannot initiate a join until refreshed. Queued joins display position, remaining lifetime and rejection/maintenance reasons. **Cancel join** or leaving the session discards late asynchronous login results. Reconnect explicitly after a terminal rejection or expiry. The server's base-map fingerprint must match the locally selected map (normally Test world); required downloadable parks are installed after admission. Steam browsing remains in the existing multiplayer pages.
