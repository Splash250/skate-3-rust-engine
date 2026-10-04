# Resource platform extension: current contract

This work continues baseline `69ed377` on `cross-platform-dedicated-server`.
The [evidence ledger](platform-extension-evidence.md) separates implemented
capabilities from graphical acceptance and unavailable native-platform checks.

## Run the local examples

Install the [Linux build prerequisites](../LINUX.md) and
[browser prerequisites](browser-interfaces.md) first. The default examples
include the inventory browser. Build once:

```sh
./BUILD.sh --browser
```

Start the server in one terminal:

```sh
cargo run --locked -p skate-server -- --test-world --max-players 64 --resources resources/platform-examples.json
```

Launch each client from another terminal:

```sh
./PLAY.sh --connect 127.0.0.1:31030 --test-world
```

The shared-object example provides two instance rooms with pushable crates and server-approved portals. The inventory opens a local browser page; authenticated purchases require the [local account setup](accounts-and-administration.md). Anonymous sessions see a sign-in explanation.

Optional examples are already granted in this configuration. At the server
console, `ensure community-park` mounts the original downloadable park;
`ensure verified-course` adds its public/private checkpoint and pickup rules;
`ensure voice-room` adds radio controls; and `ensure presentation-demo` adds
replicated clips, mascot appearance and a hat attachment. Enable the presentation
resource only after adding its `engine.animation` grant to the client's
origin-specific `grants.json`, as described in its
[README](../../resources/presentation-demo/README.md). Voice still requires each
client's `--voice` opt-in. See [resource worlds](resource-worlds.md),
[verified competitions](verified-competitions.md) and [voice](voice.md) for
controls, configuration and limits.

The JavaScript client draws the existing engine UI and calls a Lua export which
calls a JavaScript export. The server runs the same dependency chain. The SQLite
example is operated through the local server console:

```text
command progress_award test 100
command progress_buy test
command progress_inspect test
restart persistent-progression
quit
```

The account label `test` is an administrator-selected database key. It is not an
identity authentication mechanism. See [backend API](backend-services.md) and
[resource authoring](../../sdk/RESOURCES.md). The client still needs its existing
prepared character/animation assets; no retail assets are in these examples.

## Movement and instance boundary

`resource.teleport(player, {position={x,y,z}, heading=radians,
velocity={x,y,z}, instance=number})` is a server-only, `resource.teleport`-granted
operation. The Rust host validates finite bounded values and admission, then
issues a new movement epoch. The owner clears pending impulses and waits for
native travel before publishing movement. Observers retire body/pose histories;
old packets cannot move the new incarnation or create a map-wide collision sweep.
The one-second discontinuity acceptance loophole is removed.

Dedicated movement/pose, player roster, presentation and player collisions/shoves
are filtered by instance. [Shared primitive entities](shared-entities.md) use server-side Rapier simulation and scoped interest snapshots. Resource/player/instance/entity state and server events have explicit visibility scopes. Instance changes retire previous queues, activation epochs and client state. Voice proximity/radio routing also filters instances and admitted identities; socket/codec/synthetic-device tests pass. Two native Linux clients have exercised actual shared-object contact, private-instance collider/replica retirement and public-instance return. Physical microphone and native Windows checks remain separately tracked.

Native recovery requests return to the server's stored spawn/latest approved
travel destination instead of sending arbitrary client coordinates. Required resource worlds supply a validated server-selected spawn and terrain collision for shared simulation and version1 competition checks. Without a required world, the initial spawn is derived from the admitted owner's first validated observation. Coordinate validation alone is not world-geometry validation. Stock trick scores,
landings and detailed articulated skating outcomes remain owner-reported during
ordinary dedicated play. Opt-in [native-input-v1 competitions](native-skating-authority.md)
use a trusted headless engine companion, bounded controller streams and full-state
client replay in a solitary instance. See its explicit world/interaction limits
and the evidence ledger for accepted native outcomes.

Dedicated wire framing changed for movement epochs, chunked rosters and
application acknowledgements carrying both endpoints' movement epochs. These
acknowledgements cannot suppress peer metadata replay after an instance return.
Legacy peer-hosted acknowledgement framing remains unchanged. Server
and client must be built from the same version. Legacy peer sessions keep their
existing handshake and limits. Direct UDP still requires no Steam.

## Bounded transfer and configuration

The server's resource JSON accepts `network_budgets`, `runtime_limits`,
`content_limits` and `http_origins`. Omitted values use defaults. Invalid values
fail before hosting. Source content identity remains based on immutable bytes,
not on local limits. The client retains its own ceilings and validates advertised
content before activation.

| Setting | Default | Hard maximum |
| --- | --- | --- |
| Dedicated players (`--max-players`) | 16 | 64 |
| Network event/state JSON value | 16 KiB | 256 KiB |
| Pending event/large transfer count per connection | 64 | 256 |
| Queue bytes per connection | 1 MiB | 16 MiB |
| State keys per resource | 256 network | 4096 |
| Canonical network state bytes, all resources | 1 MiB | 16 MiB |
| Network resource count | 128 | 256 |
| Events per second per connection | 60 | 1000 |
| Content file | 64 MiB | 256 MiB |
| Content set | 256 MiB | 1 GiB |
| Files in content set | 16384 | 65536 (also 2 MiB metadata bound) |
| Lua/JS storage file per namespace | 4 MiB | 64 MiB |
| Storage value | 256 KiB | 4 MiB |

Decoded model/texture/audio budgets remain separate from content byte limits.
A larger downloadable file limit does not remove asset-decoder validation.
The server clamps runtime payload/state-key limits to its transport limits.
Other runtime counts, memory and instruction budgets are documented in the SDK.
A JavaScript resource has a QuickJS engine plus its bounded Lua host bridge.

Ordinary values up to 384 bytes use the small-event queue; larger values use a
separate acknowledged lane with 192-byte binary chunks (hex encoded in bounded
application records). Partial transfers, queue bytes and counts have hard limits.
Network-layer `Client::emit_large` returns a transfer ID; `cancel_large` sends an
acknowledged tombstone before subsequent transfers. Disconnect/reconfiguration
retires incomplete transfers. Small events may overtake large events; callers
needing ordering must use one lane or explicit application request IDs. All state
updates use the ordered large-message lane, including small replacements.
The explicit script API adds local request keys, progress, cancellation and deadlines; see [large resource messages](large-messages.md).

Movement is scheduled before resource application traffic. At 64 configured slots,
the bounded dedicated aggregate movement budget is 4 MB/s; per-link scheduling
has independent fairness cursors so some actor/recipient pairs cannot starve.
This is a simulated-client acceptance target, not a measured 64-window renderer
or WAN voice/browser/shared-physics capacity claim.

Example administrator configuration additions:

```json
{
  "network_budgets": {"value_bytes": 16384, "pending": 64, "queue_bytes": 1048576},
  "content_limits": {"max_file_bytes": 67108864, "max_set_bytes": 268435456},
  "runtime_limits": {"max_storage_bytes": 4194304},
  "http_origins": {"my-resource": ["https://example.org", "http://127.0.0.1:8080"]}
}
```

These are additions to a normal resource configuration, not a complete server
file. HTTP grants require both a requested/granted `resource.http` capability and
an exact configured origin. Configuration paths are server-private.

## Resource diagnostics

Use `metrics RESOURCE_ID` in the server console for the current or most recently
stopped generation's measurements. The authenticated administration status also
includes compact measurements for up to16 resources and an omitted count.
Snapshots account for serialized JSON bytes, including escaping, and report
omitted entries if logs or diagnostics exceed the administration budget. Runtime
failures log the resource, generation and reason once before retiring its content.
Retained runtime error text is bounded to2048 UTF-8 bytes, with at most128 host
diagnostics and one pending export failure per resource; repeated caught errors
still increment error metrics.
Set `SKATE_RESOURCE_DIAGNOSTICS=1` when launching a graphical client to log its
resource measurements every5 seconds. The shared host API exposes the same
structured `runtime_metrics()` values for tools and tests.

Measurements include invocation/error counts, last failure/phase, inclusive
wall time, calling-thread CPU time on Linux/Windows, consumed execution-budget
units, Lua/QuickJS heap bytes and managed worker resident bytes where available.
Wall time includes nested exports and managed IPC waits. Calling-thread CPU
excludes managed worker CPU; overlapping nested timings must not be added as
process totals. Missing platform measurements are null, not zero. Memory is
runtime-specific, not the entire renderer or shared engine. Restart resets a
resource's measurements for its new generation.

Queued output counts and admission-accounted bytes show host backpressure;
they are not sent network bytes. Dedicated transport counters and
`Host::voice_metrics()` separately report actual scheduling and packet totals.
Server logs name resource owners, generation failures and backend operation
errors. A backend completion arriving after retirement cannot call a new owner.

## Compatibility and exclusions

Lua and JavaScript are real embedded runtimes sharing lifecycle, grants, events,
state, storage and dependency exports. Cooperative Lua waits, finite vectors,
timers and a documented Cfx-style subset are supported; JavaScript exports are
synchronous JSON operations, not Node.js modules or Cfx binary compatibility.
See [Cfx comparison](resource-compatibility.md). C# executes compiled managed source in an isolated companion; Linux integration tests pass and native Windows remains unverified. [Browser UI](browser-interfaces.md), shared dynamic entity simulation and [authenticated accounts](accounts-and-administration.md) have real implementations with focused tests; their remaining acceptance checks are tracked separately. Linux voice through synthetic devices, required-world transitions, native rail registration, placement/export and version1 cosmetic animation have passed their focused integration checks, including two native graphical clients for maps, shared objects, instances and presentation. See the evidence ledger for exact acceptance status and unresolved checks.
GTA binaries/assets, GTA vehicle/weapon/ped natives and arbitrary FiveM resource
compatibility are deliberately outside the requested product target.
