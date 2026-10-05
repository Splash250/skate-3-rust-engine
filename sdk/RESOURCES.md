# Script resources (resource API 1)

A server resource is a directory containing `resource.json`. Resource API 1
selects and scopes scripts; the existing client engine SDK remains API 2.
Resources support Lua, embedded JavaScript and process-isolated C# source through
the same serializable host boundaries. C# requires a trusted .NET worker and
platform isolation prerequisites described below. Python, FiveM natives and
native DLL/SO resource plugins are not supported.

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
current side's scripts, in manifest order. Each resource gets an isolated script
VM on each side; JavaScript uses a resource-private QuickJS runtime and C# a
private managed worker process on each side that has selected C# sources. An
empty side retains its resource generation without requiring a managed runtime.
Lua scripts can register handlers at top level and return the
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

Event/state payloads default to 16 KiB of serialized JSON each. Hosts may select a
smaller negotiated transport budget or raise the runtime ceiling to 256 KiB. Keys and event
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
| `engine.animation` | Native graph controls and version 1 cosmetic clips, compatible skins and bone attachments. |
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


## Cooperative Lua compatibility

The resource VM supports `CreateThread(function)`, `Citizen.CreateThread`,
`Wait(milliseconds)` and `Citizen.Wait`. Threads first run on the next resource
update. `Wait(0)` yields until the following update; longer waits use the resource
simulation clock. Each due thread runs once per update, in creation order. Paused
simulation does not advance waits. Stopping, restarting, failing or disconnecting
the resource destroys its suspended threads. `Wait` is valid inside a thread,
including inside `pcall`/`xpcall`; it cannot suspend an ordinary engine callback.

```lua
CreateThread(function()
    while true do
        resource.emit("heartbeat", {milliseconds = GetGameTimer()})
        Wait(1000)
    end
end)
```

`pcall`, `xpcall`, `coroutine.create`, `coroutine.resume` and `coroutine.wrap`
handle normal Lua errors. Instruction exhaustion always escapes these wrappers
and retires the resource. Main callbacks, timer callbacks and all threads resumed
in one update share an instruction budget. New coroutines inherit the same VM
hook, including manually resumed nested coroutines. A yielding thread never gets
a new instruction allocation within the same update. Memory remains VM-bounded.

Lua's native `string.find`, `match`, `gsub` and each `gmatch` iterator step
reserve a conservative estimate of search and backtracking work from that same
budget before matching. Large ambiguous patterns can exhaust the budget even
when a particular input would match quickly. Plain `find(...,true)` and terminal
token scans such as `%w+` use a linear estimate. This closes the native matcher
path that otherwise runs without Lua instruction hooks; protected calls cannot
suppress exhaustion.

`vector2`/`vec2`, `vector3`/`vec3`, and `vector4`/`vec4` construct finite Lua tables
with named `x`, `y`, `z`, `w` components, componentwise arithmetic, scalar
arithmetic, equality and length (`#v`). They serialize as ordinary JSON objects.
They are not Cfx's patched Lua vector datatype: `type(v)` returns `"table"`,
components are mutable and engine APIs expecting positional arrays still require
`{v.x, v.y, v.z}`. Hash literals, GTA natives and Cfx-specific syntax remain absent.

Existing aliases include `RegisterNetEvent(name, callback)`,
`AddEventHandler(name, callback)`, `TriggerEvent(name, payload)`,
`TriggerServerEvent(name, payload)`, `TriggerClientEvent(name, recipient, payload)`,
`SetTimeout(milliseconds, callback)`, `ClearTimeout(key)`,
`exports(name, callback)`, and `exports[dependency]:name(payload)`.
Events and exports carry one serializable payload; use an array/object to group
arguments. `RemoveEventHandler(name)` removes handlers for that name in this
resource. Network registration currently requires its callback at registration.
Timers and threads do not cross generations. `GetGameTimer()` returns elapsed
resource simulation milliseconds. See the editor declarations in
[resources.lua](resources.lua).

These semantics were checked against the official Cfx documentation for
[CreateThread](https://docs.fivem.net/docs/scripting-reference/runtimes/lua/functions/Citizen.CreateThread/),
[Wait](https://docs.fivem.net/docs/scripting-reference/runtimes/lua/functions/Citizen.Wait/)
and [Lua exports](https://docs.fivem.net/docs/scripting-manual/runtimes/lua/).
This compatibility layer is an original implementation.

## Runtime and persistence budgets

Embedding hosts may construct `Host::new_with_limits(..., RuntimeLimits)`.
`Host::new` retains safe defaults. Limits are validated before a VM starts;
zero limits, excessive individual limits and count/payload combinations over the
aggregate ceiling fail with diagnostics. Runtime ceilings never grant a resource
capability, and do not override a lower transport negotiation.

| Budget | Default | Maximum configurable ceiling |
| --- | --- | --- |
| Installed resources | 128 | 1024 |
| Event / replicated state value | 16 KiB | 256 KiB |
| State keys per resource | 1024 | 4096 |
| JSON persistence per resource/side/source | 4 MiB | 64 MiB |
| Persistence value / service request or result | 256 KiB | 4 MiB |
| Persistence keys | 1024 | 16384 |
| Resource operations per callback | 128 | 4096 |
| Event handlers | 64 | 1024 |
| Cooperative scheduled threads | 64 | 1024 |
| Queued local events | 256 | 8192 |
| Queued outputs | 4096 | 8192 |
| Aggregate output JSON and envelope budget | 16 MiB | 64 MiB |
| Lua heap per VM | 16 MiB | 128 MiB |
| Approximate Lua instructions per callback | 800,000 | 10,000,000 |
| Managed process memory (`managed_memory_bytes`) | 256 MiB | 1 GiB |
| Managed callback deadline (`managed_callback_timeout_ms`) | 100 ms | 1000 ms |
| Managed compile/start deadline (`managed_startup_timeout_ms`) | 10 s | 30 s |

The memory floor is 1 MiB and the instruction floor is 64,000. Event/state queue
count multiplied by payload capacity cannot exceed 64 MiB per category. Timers
retain their independent 64-entry bound. Export nesting remains limited to 16,
JSON nesting to 32 and engine command queues to 128 per callback. Bound values
before persisting; failed writes leave the prior in-memory and disk value intact.
Existing JSON files above a newly lowered limit fail startup instead of truncating
data. JSON storage remains synchronous; use asynchronous database services for
frequent writes, transactions, concurrent updates and migrations.

## Server teleport approval and asynchronous services

`resource.teleport(player, destination)` requires `resource.teleport` and is
server-only. `player` is a canonical nonzero decimal connection ID string.
`destination` contains `position={x,y,z}`, optional `heading`, `velocity={x,y,z}`
and numeric `instance` (unsigned 32-bit). Position components are bounded to
±100000, velocity components to ±200, and heading to ±1000. The dedicated host
validates and applies the approval; enqueueing a command does not establish that
the player exists or acknowledged it. Resource code must authorize gameplay
policy before approving a destination.

Optional `restore_on_stop=true` grants a temporary travel lease to this resource
and generation. The dedicated host captures the current accepted position and
instance (zero return velocity/heading) before moving the player. At owner stop,
failure or replacement it restores that destination through normal teleport and
resource readmission. At most 64 leases exist. Another owner cannot acquire a
competing lease. An ordinary successful teleport consumes the lease; external
movement epochs, instance changes and disconnects invalidate it. Changing the required-world identity/digest disconnects still-leased actors so reconnect can use the new approved spawn and public instance; saved old-world coordinates are never applied to the new map. Trusted course
and native verifier baseline resets in the same instance retain the return
point, as do controlled same-world resource readmission epochs. Restoration waits for content readiness rather than bypassing admission.
An owner may call `resource.teleport(player,{restore_previous=true})` to return
its existing lease. This variant accepts no destination fields, never touches
another owner's lease, and may be queued while the actor is temporarily absent
from admitted player snapshots. Return requests and retired-owner restoration
wait up to 30 seconds for readiness, then disconnect that connection if it still
cannot safely return. A missing or stale lease is a no-op.
Lease ownership is host state, independent of `on_unload`; resources must never
rely on queued unload teleports surviving cleanup. Process restart expires the
connection and lease together. This field has the same meaning in Lua,
JavaScript and C# destination objects.

`resource.services.submit(key, operation, timeout_ms)` and `.cancel(key)` are
server-only, asynchronous host requests. Keys contain 1..64 ASCII identifier
characters; timeout is 1..120000 milliseconds. SQL operations (`kind="query"`,
`"transaction"`, `"migrate"`) require `resource.database`; `kind="http"` requires
`resource.http`. HTTP destination allowlists and database filesystem namespaces
are enforced again by the server service host. No credentials belong in public
scripts. Completion is a local `service_result` event:

```lua
resource.on("service_result", function(completion, sender)
    assert(sender == "0")
    -- completion.key identifies the request; completion.result is the host result.
end)
resource.services.submit("inventory", {
    kind="query",
    statement={sql="SELECT quantity FROM inventory WHERE item = ?",
               params={{type="text", value="board"}}}
}, 2000)
```

Request and result JSON use the configured persistence-value byte bound.
Cancel requests address only this resource's request key. Completions include
live resource generation checks, so stop/restart cannot deliver an old result to
a new VM. Registering the completion handler also requires `resource.events`.


## JavaScript resources

Use `"language":"javascript"` with explicit `.js` scripts in the existing manifest
script lists. Scripts are classic ECMAScript scripts evaluated in manifest order;
modules/imports, Node.js globals/packages, browser DOM, `fetch`, OS/process/file
access and native addons are not installed. Shared scripts execute independently
on both sides. No separate Node.js installation is needed. Linux execution is
covered by real runtime tests; native Windows execution remains unverified.

`resource` exposes the same metadata, capabilities, state, JSON persistence,
events, network sends, commands, services and synchronous dependency exports as
Lua. Names are `on`, `onNet` (also `on_net`), `emit`, `send`, `off`, `export`,
`call`, `command`, `players`, `teleport`, `state.get/set`, `storage.get/set` and
`entities.command` (also `entity`) and `services.submit/cancel`. Integer identities remain decimal strings; JavaScript
numbers must never carry connection/generation IDs. State and storage deletion
use `null` or `undefined`. Passing undefined nested fields, cycles, nonfinite
numbers, functions or BigInt as payloads fails validation.

Register lifecycle callbacks with `resource.lifecycle({on_load(){...},
on_update({dt}){...}, on_unload(){...}})`. The standard Lua lifecycle callback
names are supported. Multiple registrations compose in registration order.
`setTimeout(callback, milliseconds)` / `clearTimeout(id)` and
`setTick(callback)` / `clearTick(id)` are resource-owned and simulation-clock
based. Work created during a timer/tick dispatch runs on a later update.
Each timer/tick category is bounded by the configured thread count.

Cfx-style JavaScript aliases include `on`, `onNet`, `emit`, `emitNet` and
`exports(name, callback)` / `exports[dependency][name](payload)`. On the server,
`emitNet(name, recipient, payload)` accepts a decimal recipient string or `-1`
for broadcast. On clients it accepts `(name, payload)`. Events/exports carry one
JSON payload; exact dependencies and declared export names remain required.

```js
resource.export('preview', payload => ({points: payload.completed * 25}));
resource.lifecycle({on_load() {
    const result = resource.call('lua-rules', 'preview', {completed: 3});
    resource.state.set('preview', result);
}});
```

Callbacks and exports return synchronously; returned data must be plain JSON.
Returning a Promise or a custom class instance fails validation. Start asynchronous
work through completion events or microtasks and return `undefined`.
Promise microtasks run at the end of script/callback execution, with at most 256
jobs per dispatch. An unresolved promise does not block a gameplay tick; use
service completion events for I/O. Unhandled Promise rejections fail the resource;
a rejection handled in the same microtask turn remains valid. Resource stop drops
its interpreter, timers, pending jobs and host registrations. HTTP/database grants
are checked in the common host; JavaScript cannot gain ambient networking by
modifying the exposed metadata.

Client engine access includes `sdk.ui.text/remove/menu/canvas`, `sdk.log`,
`sdk.readText` for declared text assets, and `sdk.submit(command)` for the same
validated engine command schema described in [GENERAL_API.md](GENERAL_API.md).
Other Lua convenience wrappers are not automatically reproduced in JavaScript.
Headless scripts do not get client UI; submitted engine commands still require
capabilities and the appropriate side.

QuickJS uses the configured `lua_memory_bytes` value as its own heap bound, in
addition to the private Lua host adapter heap, and a fixed 256-KiB stack bound.
Its uncatchable interrupt consumes the resource's shared instruction budget;
QuickJS interrupt units are engine checkpoints, not Lua's 1000-instruction units.
Promise jobs also consume units, preventing tiny recursive jobs from bypassing
execution limits. Cross-language exports preserve each callee's independent
budget and the existing maximum dependency-call depth. Rust/JSON allocations,
output queues and persistence are bounded separately from interpreter heaps.

The redistributable [cross-language example](../resources/cross-language-demo/)
runs JavaScript → Lua → JavaScript on the shared host and includes client UI,
network events and restart-persistent storage. Editor types are in
[resources.d.ts](resources.d.ts).

The pinned `rquickjs`/`rquickjs-sys` 0.14.0 dependencies are MIT-licensed and bundle
the QuickJS engine. Only the `parallel` feature is enabled; native loader, OS
library and custom allocator features are disabled. These libraries introduce a
C compiler requirement already present for vendored Lua. Upstream API references:
[memory, stack and interrupt limits](https://docs.rs/rquickjs/0.14.0/rquickjs/struct.Runtime.html)
and [source/license](https://github.com/DelSkayn/rquickjs).

## C# resources

Set `"language":"csharp"` and select `.cs` files in the existing shared/client/server
script lists. The selected files compile together as C# 12 source into an in-memory
assembly inside a private .NET 10 worker. Exactly one concrete class must implement
`Skate.Managed.IResourceScript`; its `Start(Resource)` method registers callbacks.
Source order does not control type initialization. Downloaded DLL/SO assemblies
are never loaded, and the worker receives source text over IPC instead of mounting
a resource directory.

```csharp
using Skate.Managed;
using System.Text.Json.Nodes;
public sealed class Challenge : IResourceScript {
    public void Start(Resource resource) {
        resource.Export("points", value => JsonValue.Create(value!.GetValue<int>() * 25));
        resource.Lifecycle("on_load", payload => resource.Log("managed resource loaded"));
        resource.OnNet("ready", (payload, sender) => {
            if (resource.Side == "server") resource.Send("greeting", JsonValue.Create("hello"), sender);
        });
    }
}
```

The public API is implemented in
[`managed-host/Program.cs`](../crates/skate-mods/managed-host/Program.cs). It exposes
`Id`, `Version`, `Side`, `Generation`, `HasCapability(name)`;
`Lifecycle(name, Action<JsonNode?>)`, `On`/`OnNet(name, Action<JsonNode?,string>)`,
`Off(name)`, `Export(name, Func<JsonNode?,JsonNode?>)`,
`Command(name, permission, Action<JsonNode?,string>)`, `CallExport`, `Emit`, `Send`,
`StateGet`/`StateSet`, `StorageGet`/`StorageSet`, `Players`, `Teleport`, `Entity`/`Entities`,
`ServiceSubmit`/`ServiceCancel`, `Submit`, `Log`, `ReadText`, `UiText` and `UiRemove`.
Names and grants follow the common resource API. `Lifecycle` accepts `on_load`,
`on_unload`, `on_update`, `on_fixed_update`, `on_ui_update`, `on_event`, `on_settings`.
Player/generation identities remain decimal strings. `JsonNode?` values use the
same bounded JSON serialization; `null` deletes state/storage keys.

Callbacks and cross-language exports are synchronous. Tasks, threads, async/await,
arbitrary dependencies, native interop, reflection, process launch and ambient
filesystem/network APIs are rejected during semantic validation. Supported .NET
library types are primitives, strings, arrays, Math/MathF, common generic
collections/delegates, simple exceptions and JsonNode/JsonObject/JsonArray/JsonValue.
The exact allowlist lives alongside the compiler; unsupported types/members produce
a resource-specific diagnostic. Use asynchronous backend service requests and the
`service_result` event for I/O. Engine command results, including browser events,
arrive through lifecycle `on_event` with the same payload contract as Lua/JavaScript.
No Cfx/CitizenFX managed assembly compatibility is claimed.

Build the trusted worker using a .NET 10 SDK; the resource client/server needs only
the .NET 10 runtime and the published worker. Roslyn assemblies come from that SDK,
with no NuGet restore source or external service:

```sh
dotnet publish crates/skate-mods/managed-host/Skate.ResourceHost.csproj -c Release -o /absolute/package/managed-host
export SKATE_DOTNET_ROOT=/absolute/dotnet
export SKATE_MANAGED_HOST=/absolute/package/managed-host
```

`SKATE_MANAGED_HOST` defaults to `managed-host` beside the game/server executable.
Both environment paths are operator-controlled and must contain trusted binaries.
They cannot be selected by a downloaded manifest. .NET/Roslyn are MIT licensed;
preserve their upstream notices when distributing the runtime or worker.

Linux requires bubblewrap and util-linux `prlimit`, with unprivileged user/PID/network
namespaces available. The launcher mounts only the trusted runtime/worker and their
native dependencies read-only, exposes a fresh `/proc` and `/dev`, clears the
environment, drops capabilities and creates no resource/home/configuration mount.
The worker installs a synchronized seccomp denylist for process execution, ptrace,
cross-process memory, socket, namespace and memory-file syscalls before loading
resource code. Native dependencies currently include glibc, libstdc++, libgcc,
zlib/zstd and OpenSSL 3; missing isolation prerequisites fail startup.
Windows uses a capability-free AppContainer, explicitly inherited IPC handles and
a single-process Job Object with memory and kill-on-close limits. Native Windows
runtime behavior still requires validation; cross-compilation is not that evidence.

The managed heap hard limit is half the configured process memory budget. Linux
also checks process-tree resident memory while pumping IPC; Windows enforces its
Job Object memory limit. Callback/compile deadlines and bounded IPC are enforced
by the Rust parent, so catching a managed exception cannot defeat termination.
The managed memory floor is 64 MiB. Deadlines include synchronous downstream export
work. A worker failure retires the resource and its dependents through the normal
host path. The managed source policy is additional containment; modern
[.NET has no in-process untrusted-code sandbox](https://github.com/dotnet/designs/blob/main/accepted/2021/runtime-security-mitigations.md).
The detailed [worker IPC contract](../crates/skate-mods/managed-host/IPC.md) describes
the platform boundary and transport budgets.

[`managed-language-demo`](../resources/managed-language-demo/) demonstrates real
C# → Lua → JavaScript exports, private network events, persistence and client UI.
It is optional because both sides must install the managed runtime. Explicit
integration tests are run with the above environment configured:

```sh
cargo test --locked -p skate-mods --test managed_runtime -- --ignored --test-threads=1
```

## Shared entity commands

Server resources with `resource.entities` may call `resource.entities.command(value)`
or `resource.entity(value)`. JavaScript uses the same names; C# uses `Resource.Entity`.
The host bounds the command to 4 KiB and injects resource ownership/generation.
The dedicated server validates the complete command before changing the shared
simulation. Operations are `spawn`, `remove`, `impulse`, `velocity`, `pose`, `transfer`.
A client cannot use this API to choose authoritative transforms or entity ownership.

## Private event and state scopes

`resource.send(name, payload, recipient?, scope?)`, `resource.state.get(key, scope?)`
and `resource.state.set(key, value, scope?)` accept a scope object. JavaScript
uses the same argument order, and C# adds optional `JsonNode? scope` to `Send`,
`StateGet` and `StateSet`. Omitted/null scope means `{kind="resource"}`. Private
scopes are `{kind="instance",id="2"}`, `{kind="player",id="9"}` and
`{kind="entity",id="42"}`. IDs are canonical decimal strings; zero is allowed
only for instances. Extra fields and numeric scope IDs are rejected.

Server resources choose visibility. Clients can only send resource-scope events
to the server. State keys are distinct per scope, while the live-key budget
applies across all scopes of one resource. Private state disappears locally when
the transport retires its visibility, and the server removes player/entity state
when its target disappears. Instance and resource state remain until explicitly
removed or the resource stops. Scope selection does not bypass the dedicated
server's entity ownership, generation, recipient or interest checks.

`resource.entities.all()` (C# `Entities()`) returns the host's visible entity
observations, including canonical decimal identity strings, and requires
`resource.entities`. Entity commands complete asynchronously through the host;
a command's local return does not establish that a target exists or was changed.

## Voice control

Lua and JavaScript use `resource.voice.submit(operation)`; C# uses `Voice(operation)`.
Server `resource.voice` grants allow `channel {name,members}`, `remove_channel
{name}`, `mute {player,muted}` and `proximity {meters}`. Operations use a `kind`
field. Member/player IDs are canonical decimal strings; channels have at most
64 members and proximity is 1–100 metres. The server owns routing and moderation.

Client `engine.voice` grants allow `devices {}`, `configure {input_device?,
output_device?,muted,deafened}` and `transmit {pressed,channel?}`. A channel is
`resource/name`; an omitted channel selects proximity. Each operation is bounded
to 4 KiB and checked for side, capability and shape before queuing. Client scripts
can also use `sdk.submit({kind="voice",operation=...})`, JavaScript `sdk.submit`,
or C# `Submit` through the same checked path. Host `voice_result` notifications
are local events and generation checked; a successful submission means queued,
not that a microphone/device or remote recipient was available.

## Explicit bulk transfer and authoritative operations

`resource.transfer.start(key,name,payload,{recipient?,timeout_ms?})` and
`resource.transfer.cancel(key)` require `resource.events`. C# exposes
`TransferStart` and `TransferCancel`. Clients send only to the server; servers
must supply one canonical decimal recipient. Timeout is 1–120,000 ms, default 10,000.
The transfer uses the same host/negotiated payload budget as resource values
(16 KiB default, configurable up to 256 KiB), with separate bounded transport
scheduling, handles, cancellation and progress. Receiving network handlers use
the existing `resource.on_net`/`resource.onNet` and `resource.network` grant.
Local `transfer_progress` reports key, state, acknowledged/total bytes and any
error. A delivered transport acknowledgement does not prove callback success.

Server-only `resource.world.command(operation)` (C# `World`) requires
`resource.world` and allows `op:"rail_upsert"`/`"rail_remove"`, bounded to 64 KiB.
`op:"select",resource:"map-resource-id"` requests an operator-allowlisted world
rotation through the normal resource lifecycle; the host validates selection and
reports a `world_result` event. It cannot install arbitrary map packages.
Server-only `resource.competition.submit(operation)` (C# `Competition`) requires
`resource.competition` and allows `kind:"define"`, `"start"`, `"cancel"` or
`"remove"`, plus `"native_start"`/`"native_cancel"` when the operator enables
[native input authority](../docs/multiplayer/native-skating-authority.md), bounded
to16KiB. Native start takes canonical decimal `player` and1–3600 `ticks`;
completion carries the trusted native score ledger and `verified_rules:"native-input-v1"`.
The dedicated host performs complete typed world/
course validation. Neither operation accepts a resource owner/generation from
the script, and competition has no client score-submission operation. Command
completion and verified outcomes arrive through generation-checked local events.

Client [`animation version 1`](ANIMATION.md) supports rig-validated imported
clip banks, blends, cosmetic marker events, appearances and bone attachments in
Lua, JavaScript and C#. Presentation events do not establish authoritative score.

## Host diagnostics

Retained error messages are limited to2048 UTF-8 bytes. The host keeps at most128
diagnostics and one pending export fault per resource, while counting every
failed invocation in its metrics. A failed callback retires its resource and
dependents; already drained engine commands cannot run afterward.

`Host::runtime_metrics()` reports each started resource's current or most recent
generation. It measures invocation count, errors and a bounded last error,
inclusive elapsed time, actual native host-thread CPU time on Linux/Windows,
callback budget units, live Lua/QuickJS heap allocations and Linux C# process-tree
resident memory. Missing measurements are `null`, including memory after stop
and managed RSS on Windows. C# worker CPU is not part of host-thread CPU time.

Invocation counts cover startup and dispatch attempts, event/command batches and
callee exports. Elapsed time includes managed IPC and downstream calls; nested
resource measurements overlap and must not be summed as total process load.
Startup instruction usage is not sampled; budget-unit metrics describe subsequent
dispatches and exports. Queued output counts and admission-accounted bytes report
the local host queue, not sent traffic or exact allocator usage. Failure counters
survive retirement; a new generation starts fresh, and replacing the installed
set clears prior measurements. The host does not capture payloads or storage for
metrics; error text can contain values included by the script itself.

## Typed operator settings

Declare `settings` and `requires_features:["resource.settings.v1"]` in a resource
manifest. `resource.settings` must be requested and granted. Lua/JavaScript use
`resource.settings.get(key)` and `.all()`; C# uses `SettingsGet(key)` and
`SettingsAll()`. Reads are confined to the caller's resource and return copies.
There is no script-side setter. The operator owns updates and persistence.

For example, `"round_seconds":{"type":"integer","default":180,"min":10,
"max":3600,"visibility":"replicated","change":"live"}` declares a bounded
setting. Types are `boolean`, `integer`, `number`, `string`, and string `enum`
(with `options`). Integer values stay within the exact JavaScript integer range.
Strings have `max_bytes` (default256, maximum4096). At most64 keys,32KiB of
schema and8KiB of effective values are allowed. A key uses lowercase letters,
digits and underscores, up to64bytes. Unknown fields, keys and types fail closed.

Visibility is `private` (default), `replicated`, or `public`. Private definitions
and defaults are removed from downloadable manifests. Replicated values reach
admitted clients; public values may also appear in discovery. Keep real secrets
in private operator configuration, never in public defaults or scripts. Changes
are `live` (default) or `restart`. A live change invokes `on_settings` with
`{key,value}` after updating reads. A restart change is persisted as pending and
applies on the next resource generation, including dependent restarts. A failing
notification retires the resource, but the durable change remains saved.

The authenticated host reserves resource-state key `__settings`; scripts cannot
write it. The game waits for the complete current-generation settings snapshot
before running client startup callbacks. See [operator settings](../docs/multiplayer/resource-settings.md)
for host interfaces, authorization and persistence semantics.

## Profiling history

`Host::profile_snapshot()` retains metadata-only invocation spans across resource
generations, with resource/generation, callback/event/export name, available
source location, parent span, wall time and host-thread CPU. It also supplies
retained-sample p50/p95/p99 summaries and portable Chrome/Perfetto trace JSON via
`.chrome_trace()`. Defaults retain4096 completed spans for60seconds; maximums are
8192spans and300seconds. Disable or tune using
`Host::configure_profiling(enabled,capacity,retention_ms)`. Existing aggregate
metrics continue while timeline capture is disabled.

Only exclusive host-thread CPU may be added across nested spans. Managed worker
process CPU is a separate measured field on IPC spans; it includes worker
background threads and excludes the host. `ipc_receive_wait_us` is blocked receive
time and includes worker execution; it is not pure queue wait. Local event
`queue_wait_us` measures enqueue-to-dispatch time before the span. Unavailable
measurements are null. `@host` dispatch spans correlate callback costs with
resource dispatch stalls. Trace metadata contains no payloads, settings values,
storage contents, full source or script error text. See
[profiling](../docs/multiplayer/resource-profiling.md) for use and measurements.

## In-game interfaces and administration

Feature IDs `resource.interfaces.v1`, `browser.surface.v1`, `engine.photos.v1` and
`resource.admin.v1` can be declared in `requires_features`. The engine's generic
registry/input/composition APIs are described in [GENERAL_API.md](GENERAL_API.md)
and [resource interactions](../docs/multiplayer/resource-interactions.md).

`resource.admin` is an explicit grant for a narrow native administration bridge,
not authority to bypass a player's roles. Client resources send
`resource.send('__host_admin',{seq='correlation',action={kind='permissions'}})` and
register `resource.on_net('__host_admin_result',callback)`. `permissions` accepts an
optional `check={"custom.manage"}` list of up to 128 permission names, each at most
64 ASCII letters/digits or `_.:-*`. Only currently permitted requested names and
the built-in dashboard permissions return; no account secrets or role internals
are exposed. Other supported kinds
are `status`, `settings_read`, `settings_set`, `resource_start`, `resource_stop`,
`resource_restart` and `profile_read`, with the existing typed action fields.
Only sender `"0"` is accepted for results. Correlation IDs are bounded 32-byte
letters/digits/underscore/hyphen strings; replies contain `{seq,ok,value,error}`.

The native server intercepts the reserved request before resource callbacks and
uses the actual admitted sender. Generation, requested/granted capability, live
verified session and the action's permission are checked. Execution uses the
existing audited administration queue, and original permission/session are checked
again before private delivery. Limits are four pending requests per actor, 64
overall, eight ingress requests/second per actor and 15 KiB response data. Large
results become actionable errors, not partial private settings. Pages never receive
account credentials or unrestricted execution. Close a dashboard by retiring its
pending UI correlations; a stale response must not repopulate a new screen.

The RP profile includes `interaction-policy`, `master-menu`, `phone`, `phone-calls`,
`inventory-ui`, `admin-dashboard` and `park-guide`, with exact dependencies and
explicit grants in [rp-server.json](../resources/rp-server.json).

### Authorizing plugin-specific server handlers

Declare and grant `resource.authorization`, with required feature
`resource.authorization.v1`. Lua and JavaScript use
`resource.authorized(sender, permission)`; C# uses
`resource.Authorized(sender, permission)`. The result is a boolean derived from
the existing live verified account session. It is false on clients, without the
capability, without verified login, for disconnected actors, and after permission
revocation. A permission name is 1–64 ASCII letters/digits or `_.:-*`; actor IDs
are canonical nonzero decimal strings. No role array or identity supplied by a
client payload is trusted, and this call does not access SQL or expose credentials.

```lua
resource.on_net("diagnostics_test", function(request, sender)
    if not resource.authorized(sender, "calls.test") then return end
    -- Validate and rate-limit this specific bounded operation here.
    local result = run_bounded_invariant_test()
    -- Recheck before returning private data; roles can change during work.
    if resource.authorized(sender, "calls.test") then
        resource.send("diagnostics_result", result, sender, {kind="player", id=sender})
    end
end)
```

Always use the handler's actual sender argument. A visibility check in a menu
does not authorize a handler. Each private read, mutation and test operation must
check its own permission. Recheck before returning results, including after an
export or asynchronous completion, and scope results
to that same actor and request generation. See the shipped
`resources/call-diagnostics` dashboard for separate read/test permissions.
