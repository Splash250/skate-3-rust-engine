# Lua resources (resource API 1)

A server resource is a directory containing `resource.json`. Resource API 1
selects and scopes scripts; the existing client engine SDK remains API 2.
Resources use Lua first, with serializable host boundaries intended for later
language runtimes. JavaScript, C#, Python, FiveM natives and native DLL/SO
plugins are not supported resource languages.

## Manifest

```json
{
  "format": 1,
  "api": 1,
  "id": "my-challenge",
  "version": "1.0.0",
  "language": "lua",
  "shared_scripts": ["shared.lua"],
  "client_scripts": ["client.lua"],
  "server_scripts": ["server.lua"],
  "files": ["ui/title.txt"],
  "dependencies": {"skate-rules": "1.0.0"},
  "exports": [],
  "capabilities": ["resource.network", "resource.state", "engine.ui"]
}
```

Dependencies use exact versions. Shared scripts execute first, followed by the
current side's scripts, in manifest order. Each resource gets an isolated Lua
VM on each side. Scripts can register handlers at top level and return the
existing callback table (`on_load`, `on_update`, `on_fixed_update`,
`on_ui_update`, `on_event`, `on_unload`). Use local variables where possible;
globals deliberately shared between a resource's files remain within that VM.

List downloadable non-script assets under `files`. Client/shared script files
are automatically public; server scripts are private. Do not list a file twice
or overlap server and public paths. Paths are relative, canonical lowercase
ASCII with `/` separators. Absolute paths, traversal, hidden paths, backslashes,
Windows device names, trailing spaces/dots and case aliases are rejected.
There are no manifest globs or executable manifest expressions.

Never put credentials in client/shared scripts or public assets. A dependency
does not grant direct access to another resource's files, state or storage.

## Resource API

The global `resource` is also available as `sdk.resource`. Metadata includes
`id`, `version`, `side`, `generation` and `grants`. A grant records permission;
`sdk.capabilities` describes compiled engine features. Neither should be treated
as a claim that a remote client is trustworthy.

| Function | Contract |
| --- | --- |
| `resource.on(name, function(payload, sender))` | Register a local handler for this resource. Requires `resource.events`. |
| `resource.emit(name, payload)` | Emit locally within the same resource. It does not send a packet or impersonate another resource. |
| `resource.on_net(name, function(payload, sender))` | Explicitly allow a network event. Requires `resource.network`. |
| `resource.send(name, payload)` | Client sends to server; server broadcasts to its clients. |
| `resource.send(name, payload, recipient)` | Server targets an admitted player. Clients cannot select another sender. |
| `resource.state.get(key)` | Read this resource's replicated state. Requires `resource.state`. |
| `resource.state.set(key, value)` | Server replaces an owned state value. Client writes are rejected. |
| `resource.storage.get(key)` / `.set(key, value)` | Read/write bounded JSON data within this source/resource/side namespace. Requires `resource.storage`. |
| `resource.export(name, function(payload))` | Register a declared exported function. Requires `resource.exports`. |
| `resource.call(dependency, name, payload)` | Synchronously invoke a live declared dependency export on the same side. Values cross the VM boundary as serialized data. |
| `resource.command(name, permission, function(args, actor))` | Register a server command with a permission requirement. Requires `resource.commands`. |
| `resource.players()` | Read the current host player observations; on the dedicated server these are bounded owner-reported movement/gameplay observations. |

`resource.players()` returns the host's current observation list with string
`id` values. Server entries contain `position` and the dedicated `gameplay`
wire fields; client entries reuse the existing SDK skater observations (flat
fields such as `mode`, `trick`, `landing_seq`, `score`, and `local`). Remote
entries represent received observations, not independently verified simulation.

Event/state payloads are limited to 384 serialized bytes each. Keys and event
names are bounded ASCII names. Keep messages compact and never transfer assets
through events. Handler queues, state count, instruction execution, Lua memory,
export recursion and host commands also have runtime limits. JSON nesting is
limited to 32 levels; nonfinite values and cycles are rejected. Synchronous
native physics queries require `engine.physics` and consume the resource
operation budget (128 per callback). They return nil inside dependency exports, resource event/command callbacks
and unload callbacks because a caller's physics bridge must not expose another
resource's bodies;
return calculation data from an export and query physics in the caller's fixed
callback. `sdk.assets.objects` requires `engine.graphics` and reads bounded GLB
name metadata only; it does not import external files or decode images. Lua
`__gc` finalizers are unsupported for resource scripts; use `on_unload`. Ordinary
Lua garbage collection remains enabled. Failed scripts
must release owned callbacks, timers, objects and overrides; an unload callback
is a convenience, not the cleanup guarantee.

The handler's `sender` is supplied by the host: a connected player's identity
for client-originated requests, and `"0"` for local/server-originated dispatch. All player/sender/actor IDs,
recipient arguments and the `resource.generation` metadata are canonical decimal
strings, preserving the full unsigned 64-bit identity without Lua floating-point
rounding. Keep them as strings; do not convert them with `tonumber`.
Do not trust a `player`, `sender`, `score` or permission field supplied in the
payload. On clients, accept server-only events only from sender `"0"`. On servers,
check membership, allowed state transitions, cooldowns and relevant observations
before applying a request. Event registration is not gameplay authorization.

```lua
resource.on_net("join", function(payload, sender)
    if sender == "0" or type(payload) ~= "table" then return end
    -- Validate payload and the server's current player/round state here.
    resource.send("joined", {accepted = true}, sender)
end)
```

Replicated state is explicit replacement, not a shared mutable Lua table.
Changing a field of a value returned by `state.get` does not publish it. Only
the server sets canonical resource state. Resource generations distinguish live
instances so a stopped/restarted resource cannot accept an old instance's event.
Persistent storage is separate from immutable downloaded content and survives
ordinary stop/restart; it is never automatically distributed to clients.

Use `sdk.time.after(key, seconds, callback)` and `sdk.time.cancel(key)` for
bounded VM-owned timers. Simulation time does not advance through a paused
client's simulation callbacks; use `on_ui_update` for presentation during pause.

## Engine grants and side boundaries

Request only the capabilities the resource actually uses. Server configuration
provides server grants; the client independently applies source-scoped policy.
Downloading bytes does not authorize camera/input/player control.

| Capability | Existing client interface |
| --- | --- |
| `engine.ui` | `sdk.ui`: menus, text and canvas UI. |
| `engine.audio` | `sdk.audio`: bounded resource-owned audio. |
| `engine.graphics` | `sdk.graphics`: resource models, transforms, mesh buffers and lights. |
| `engine.physics` | `sdk.physics`: owned bodies, colliders, joints, forces and queries. |
| `engine.player` | Player movement, suspension, attachment and native body/rig controls. |
| `engine.camera` | Camera follow/watch/rig/set/capture controls. |
| `engine.input` | Mapped action overrides. |
| `engine.world` | World/volume operations. |
| `engine.animation` | Native graph controls within existing exposed boundaries. |
| `engine.inspect` | Engine observations/catalog inspection. |

Headless server scripts do not get rendering, audio, cameras, input or native
client physics execution. Use events and owned state to coordinate clients.
The dedicated server retains authority; legacy `sdk.session.claim/transfer`
must not transfer it to a player. The [engine audit](../docs/multiplayer/resource-compatibility.md#existing-engine-capability-audit)
lists real limitations, including arbitrary animation replacement and native
score-collector replacement. [GENERAL_API.md](GENERAL_API.md) documents existing
primitives and ownership; [ENGINE_API.md](ENGINE_API.md) documents observations.

Dedicated remote skater observations expose transmitted pose, approximate body
velocity, movement mode, landing/bail counters, trick labels and scores. Detailed
clean/sketchy landing flags, graph state, raw input and transmitted gameplay
cameras are not present in the dedicated wire contract. Camera watching uses
the existing body-follow fallback. Local-player engine observations remain
available; do not interpret default values for unavailable remote details as
confirmed results.

Downloaded GLB models are prevalidated on the download worker before any resource
VM starts, and must embed their buffers and images. External glTF URIs
are rejected before AssetServer loading so an asset cannot read another package
or cache policy/state file. Downloaded scenes limit their JSON description to 256 KiB and enforce
predecode image dimensions (2048 per side),
16 MiB aggregate decoded texture data (including repeated image references),
16 MiB expanded geometry allocation, bounded
accessors/primitives/nodes, and a million expanded scene vertices. Sparse
accessors, cyclic/shared-child graphs and glTF extensions are rejected. PNG
compressed profiles/text, APNG and unknown metadata chunks are also rejected
in downloaded GLBs before decoding. Animation allocation is charged per channel. Separate
explicitly listed PNG textures remain usable through the bounded mesh-buffer
texture API; dimensions are checked before decoding.

## Migrating local mods

Existing `mod.json` API 2 packages remain local mods for the original local/peer
host path. A server resource uses a separate `resource.json`, explicit grants
and side selection. Start by placing a local mod's entry in `client_scripts`,
listing its asset files, and requesting the required engine capabilities.
Keep its callback table and supported generic SDK calls.

Move shared decisions, permissions and canonical state into `server_scripts`.
Replace peer-host authority transfer and generic peer state publication with
explicit resource events and server-owned state. Put only public constants and
portable helpers in `shared_scripts`. Replace private filesystem assumptions
with resource-scoped storage. Split reusable helpers into declared dependencies
and exports. Test cleanup, reconnect and denied grants as well as normal play.

The [landing challenge](../resources/landing-challenge/) demonstrates these
pieces together. [Server setup, cache inspection and troubleshooting](../docs/multiplayer/resources.md)
cover operation. [Cfx compatibility](../docs/multiplayer/resource-compatibility.md)
documents intentional differences; an existing FiveM resource is not directly
loadable simply because it uses Lua.
