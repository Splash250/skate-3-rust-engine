# Phone calls resource

`phone-calls` is a real two-party signaling plugin for the existing dedicated
voice transport. It creates a resource-owned voice channel only after the
callee accepts, then reports connected only after the host confirms creation.
No new transport, account store or external service is used.

Grant the server `resource.network`, `resource.events`, `resource.voice` and
`resource.settings`; clients need `resource.network`, `resource.events`,
`resource.exports`, `resource.settings` and `engine.voice`. The complete
[RP configuration](../rp-server.json) includes these grants. Clients still need
local `--voice` opt-in. Hold **V** or **LB** to speak during a call. The native
host enforces the physical hold, local mute/deafen and release across focus
transitions. A resource cannot synthesize microphone consent. Calls continue
when the phone closes; they are not hands-free.

The server accepts only actual admitted sender identities and same-instance
participants. Canonical actor IDs are connection identities, not phone numbers
or account identifiers. The directory displays `Player ACTOR_ID` because the
existing player observation API does not supply a verified display name.
Disconnect/reconnect creates a new actor and ends the old call.

The shipped manifest declares two live typed settings:

- `ring_seconds`: 10–60, default 30, replicated.
- `dial_cooldown_seconds`: 3–60, default 5, private.

There are at most 16 ringing/connecting/active or retiring calls, one per player.
Call IDs contain resource generation plus an increasing bounded serial. The
recipient has a separate three-second ring cooldown. Presence expires after
15 seconds; the native client's UI update sends a heartbeat every five seconds,
including during pause. Presence reports availability only: it never grants
voice membership or bypasses the host microphone gate. The server rechecks
participants and instance membership on requests, completions and updates.
Pending channel removals retry once per second and continue consuming a slot
until confirmed, preventing channel exhaustion from leaving forgotten calls.
Host generation cleanup independently revokes every channel on failure/restart.
A client presentation must read the `snapshot` export at least once every
three seconds, including while its window is closed. This resource-owned lease
marks the client unavailable if its presentation fails or retires; it does not
name a specific UI resource. The shipped phone polls every 350 ms even when
closed. Renewing the lease cannot bypass native voice opt-in. No history, phone
numbers or contact identities are persisted.

## Client extension contract (version 1)

Declare exact dependency `"phone-calls":"1.0.0"`, export capability and grant:

```lua
local state = resource.call("phone-calls", "snapshot", {})
resource.call("phone-calls", "request", {action="contacts"})
resource.call("phone-calls", "request", {action="dial", target="123"})
resource.call("phone-calls", "request", {action="accept", id=state.call.id})
```

`request` allows `contacts`, `dial`, `accept`, `decline`, `cancel` and `hangup`.
The last four require the current call ID. Returns `{ok,error?}` for local
submission only; authoritative state arrives asynchronously in `snapshot`.
`call.state` is `idle`, `ringing`, `connecting`, `active` or `ended`; call records
also contain `id`, `incoming`, `peer`, optional `reason`, and active `channel`.
The snapshot contains bounded `contacts`, `message`, native `voice`, local
`controls`, and enumerated `devices`. Never show a submitted request as connected.

`request {action="devices"}` refreshes actual native devices.
`request {action="voice",muted?,deafened?,input_device?,output_device?}` applies
local phone controls; device strings must match current enumeration or be empty
for the OS default. Local keyboard mute/deafen continue to take precedence.
Native opening/error status is visible in `snapshot.voice.device`. Device lists
contain at most eight names and 512 UTF-8 bytes per kind, so the combined export
remains bounded alongside a full player directory. If the list is truncated,
choose System default to use an omitted device selected through the OS.

Server events are private targeted messages. Forged payload `sender`, unknown
call IDs, wrong-role actions, simultaneous dialing, stale acceptance and
late completions do not authorize a transition. Event schemas are deliberately
small; no command strings, eval or arbitrary filesystem operations exist.

## Server diagnostics extension

The `diagnostics` and `invariant_test` exports inspect actual server call state
without modifying calls or voice membership. The separate
[call-diagnostics](../call-diagnostics/README.md) plugin exposes them through
an in-game dashboard with independent `calls.diagnostics` and `calls.test`
permission checks. Neither export includes participant identities.

## Evidence

```sh
cargo test --locked -p skate-mods --test phone_calls
cargo test --locked -p skate-server --test phone_calls -- --nocapture
```

The second suite executes the shipped Lua resource over actual TLS login and
encrypted loopback UDP, encodes generated PCM with Opus, decodes accepted audio,
and verifies that a nearby third party receives no call audio. It also checks
that ringing, hangup and resource restart do not grant/retain membership. This
is deterministic audio-routing evidence; it does not establish physical
microphone/speaker acceptance or native Windows behavior.
