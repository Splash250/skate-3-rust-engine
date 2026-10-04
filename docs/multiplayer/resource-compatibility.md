# Resource platform: Cfx comparison and engine API audit

Research checked on 2026-10-04 against official Cfx documentation and
`citizenfx/fivem` commit
[`e34d12cd9a39cc223548a5be1ab09f60e9183051`](https://github.com/citizenfx/fivem/commit/e34d12cd9a39cc223548a5be1ab09f60e9183051).
This platform follows the resource operating model; it does not implement FiveM
binary, network, manifest or GTA-native compatibility. Arbitrary FiveM/GTA
resources require porting. The implementation is original; no CitizenFX source
has been copied into this repository for this research.

## Compatibility decisions

This table records the implementation contract and deliberate differences.
The resource host, content service/cache, reliable dedicated channel, client
engine integration and bundled Lua challenge are implemented. Exact authoring
syntax is documented in [RESOURCES.md](../../sdk/RESOURCES.md); executable and
platform-specific evidence is recorded in [resource-validation.md](resource-validation.md).
The comparison below describes supported behavior, not binary FiveM compatibility.

| Area | Cfx behavior | Skate contract / deliberate difference |
| --- | --- | --- |
| Manifest and scripts | `fxmanifest.lua` supplies metadata, client/server/shared entries, downloadable files, dependencies and exports. Shared/client scripts are included in client content; `server_only` suppresses client downloads. [Manifest reference](https://docs.fivem.net/docs/scripting-reference/resource-manifest/) | Versioned `resource.json` with explicit file lists and distinct client/server/shared entry points. Server scripts and private configuration are excluded from client manifests/content. No executable `fxmanifest.lua`, GTA `data_file`, globbing or arbitrary custom metadata compatibility is promised. |
| Dependencies and exports | Dependencies establish load ordering; runtime exports expose functions across resources. [Manifest](https://docs.fivem.net/docs/scripting-reference/resource-manifest/), [Lua runtime](https://docs.fivem.net/docs/scripting-manual/runtimes/lua/) | The host validates exact dependency versions/cycles before activation and binds exports to live resource generations. Explicit dependency/export contracts exchange bounded serialized values. |
| Lifecycle | `start` starts a stopped resource; `stop` stops it; `restart` requires a running resource; `ensure` restarts a running resource or starts a stopped one. [Commands](https://docs.fivem.net/docs/server-manual/server-commands/#resource-commands) | These lifecycle meanings are implemented. Stops retire owned engine state even if a script fails; dependency restarts also restart active dependents. Startup resolves one dependency closure without restarting dependencies repeatedly. |
| Events | Local events and network events are distinct. Large latent events have bandwidth limits; ordinary large network events can block the channel. [Events](https://docs.fivem.net/docs/scripting-manual/working-with-events/triggering-events/) | Local dispatch and explicitly registered network events are separate. The channel derives sender identity from admitted connections and bounds bytes, rate, queues and generations. HTTP transfers assets independently of movement packets. |
| Authorization | Registering a network handler makes it reachable; scripts must validate relevant state and permissions on the server. Client checks are bypassable. [Secure your events](https://docs.fivem.net/docs/developers/server-security/) | An event name, a claimed player ID or possession of a resource download is not authorization. The host binds resource identity; Lua validates gameplay policy against server observations. A modified client can forge its own permitted inputs, so an inventory or readiness acknowledgement is not anti-cheat attestation. |
| Replicated state | Entity owners/server can normally write entity state, players/server write player state, and only server writes global state. `sv_stateBagStrictMode` can restrict replicated state writes to the server. Nested assignment is not automatically replicated. [State bags](https://docs.fivem.net/docs/scripting-manual/networking/state-bags/) | Resource state is server-owned, with explicit replacement and generation checks. Clients request changes through validated events. This is a resource-scoped state contract, not a reproduction of GTA entity state bags or OneSync ownership migration. |
| Distribution and cache | Connection configuration leads to HTTP file requests before gameplay; file hosting can be proxied. [Connection process](https://docs.fivem.net/docs/server-manual/proxy-setup/#connection-process) | Independently bounded HTTP content delivery, verified persistent content storage and activation only after all required content is ready. Content identity uses full BLAKE3 digests and portable canonical paths. Cache bytes may be shared across servers; grants and private mutable data must remain isolated. |
| Language/runtime | Cfx supports Lua, JS and C#. CfxLua adds vectors, quaternions, hash syntax and other runtime facilities. [Scripting reference](https://docs.fivem.net/docs/scripting-reference/), [Lua](https://docs.fivem.net/docs/scripting-manual/runtimes/lua/) | Lua first. Reserve language-neutral lifecycle, event and host-command boundaries for later runtimes. No CfxLua syntax extensions, JavaScript, C#, Python or downloadable native code support is claimed. |
| Files and permissions | Cfx's sandbox restricts cross-resource writes and system paths, rejects traversal/symlinks, and provides explicit permission overrides. [Sandbox](https://docs.fivem.net/docs/developers/sandbox/) | Immutable downloaded content plus a bounded resource-scoped persistence API. No general `io`, shell, arbitrary process memory or DLL/SO loading. Compiled API capability discovery and per-resource grants are separate concepts. |
| UI and game natives | FiveM exposes GTA-specific natives and browser NUI facilities. [Native introduction](https://docs.fivem.net/docs/scripting-manual/introduction/about-native-functions/), [NUI](https://docs.fivem.net/docs/scripting-manual/nui-development/) | Use Skate's existing UI/canvas/mesh/audio APIs and engine primitives. Browser NUI, GTA assets, models and natives are not compatible. Challenges, scoring policies and game rules belong in resource scripts. |

## Source observations affecting the design

The following are observations about specific upstream implementations, not a
claim that every CitizenFX subsystem follows identical rules:

- [`Resource.cpp`](https://github.com/citizenfx/fivem/blob/e34d12cd9a39cc223548a5be1ab09f60e9183051/code/components/citizen-resources-core/src/Resource.cpp#L81-L133)
  implements explicit starting/started/stopped transitions and lifecycle hooks.
- [`ResourceScriptingComponent.cpp`](https://github.com/citizenfx/fivem/blob/e34d12cd9a39cc223548a5be1ab09f60e9183051/code/components/citizen-scripting-core/src/ResourceScriptingComponent.cpp#L92-L110)
  selects shared scripts and the current side's scripts across pluggable script
  runtimes. Its [stop hook](https://github.com/citizenfx/fivem/blob/e34d12cd9a39cc223548a5be1ab09f60e9183051/code/components/citizen-scripting-core/src/ResourceScriptingComponent.cpp#L197-L206)
  destroys runtimes and clears tick registrations. Skate should additionally
  retire every resource-owned engine object/override independently of Lua cleanup.
- [`ServerResources.cpp`](https://github.com/citizenfx/fivem/blob/e34d12cd9a39cc223548a5be1ab09f60e9183051/code/components/citizen-server-impl/src/ServerResources.cpp#L745-L809)
  implements restart and ensure, with a configuration-time exception preventing
  repeated ensures from restarting dependencies. Its comment explicitly calls
  out dependent-resource restart behavior; it should not be copied as an
  unquestioned lifecycle policy.
- [`ResourceCache.cpp`](https://github.com/citizenfx/fivem/blob/e34d12cd9a39cc223548a5be1ab09f60e9183051/code/components/citizen-resources-client/src/ResourceCache.cpp#L117-L225)
  hashes content, uses digest-keyed files/index entries and records source
  metadata. That implementation uses SHA-1. Skate's new cache should use its full
  modern digest contract instead; resource names or legacy truncated fingerprints
  are unsuitable persistent identities.
- [`StateBagComponent.cpp`](https://github.com/citizenfx/fivem/blob/e34d12cd9a39cc223548a5be1ab09f60e9183051/code/components/citizen-resources-core/src/StateBagComponent.cpp#L773-L803)
  checks the incoming source against a bag's owning peer before mutation. This
  supports the general ownership design; resource scope, server authorization,
  limits and generation rejection still need independent Skate enforcement.

## Existing engine capability audit

This audit combines the existing engine primitives with their resource integration.
Client engine commands flow through the same concrete adapters under resource
ownership and grants. Server VMs omit client engine command namespaces.
The public baseline is [GENERAL_API.md](../../sdk/GENERAL_API.md),
[ENGINE_API.md](../../sdk/ENGINE_API.md) and
[skate.lua](../../sdk/skate.lua). Runtime wrappers live in
[api.lua](../../crates/skate-mods/src/api.lua), command validation and VM limits
in [vm.rs](../../crates/skate-mods/src/vm.rs), and concrete execution in
[modding](../../crates/skate-game/src/modding/mod.rs).

| Area | Verified existing primitives | Boundary / integration requirement |
| --- | --- | --- |
| Entities/lifecycle | Resource-keyed Rapier bodies, colliders, joints, mesh/light entities, removal, transforms and scene nodes. Game retirement removes owned bodies, joints, graphics, overlays and menus. | These are engine objects with ownership, not arbitrary Bevy world access. The resource host feeds retirement through the existing cleanup path. |
| Physics/skating | Forces, impulses, velocities, motors, springs, raycasts, geometric player overlap; native local-player body observations/impulses; joint/part overrides; teleport, suspension and attachment. | Detailed native controls are local-player operations. The headless server does not run the full retail terrain/limb simulation; server policy must account for Hybrid Authority. Applied physical impulses are not undone by unloading a script. |
| Animation | Animation observations, graph catalogs and eligibility gates; presentation controls in the existing SDK. | No arbitrary pose injection, animation-blend replacement, native graph operation replacement or native rig-topology creation. Gates do not force native state transitions. |
| Camera/input | Follow/watch/set/rig/capture cameras, pad/key/action observations, bounded mapped-action overrides. | Presentation/client capabilities only. Overrides have ownership and are restored on stop/disconnect. Raw input remains distinct from mapped actions. |
| UI/audio | Owned text overlays, canvas drawing, pause menus, custom sections, mesh buffers, lights, audio preload/play/update/stop. | No web browser/NUI host. Resource assets resolve within verified packages, with image/geometry/audio limits. Headless scripts do not receive presentation commands. |
| Maps/world | Map observations, resource geometry, models, collision and volume queries. | No arbitrary map-byte rewriting or general resource-driven map transition API. Distributing a file alone does not make it a usable engine map or authorize retail-asset redistribution. |
| Scoring/rules | Trick announcements, confirmed landing/bail counters, settled trick metadata and scoring catalogs. Existing Lua examples implement game rules. | No native collector replacement. Dedicated clients' score/trick observations are untrusted presentation inputs, not a proven competitive leaderboard. Rules and validations must be server-authored. |
| Networking | Existing local-mod state publication/entity replication and peer-session operations; dedicated UDP separately admits peers and routes validated movement/gameplay/effects. | Dedicated clients disable local mod discovery, execute only verified server-selected resources and suppress legacy peer mod replication/session authority transfer. Resource control records come only from the authenticated host actor. |
| Persistence/capabilities | Local mod settings/preferences, package-bounded reads, compiled feature discovery. | The new resource host supplies bounded isolated JSON persistence and separate server/client grants. Mutable state is scoped by source/resource/side; cached content can be reused across sources. |
| Runtime containment | Separate Lua VMs, 16 MiB Lua memory limit, instruction budget, validated commands and bounded command queues. VM-owned timers/callbacks die with the VM. | Native commands, graphics/audio decoding, network queues, downloads and disk storage have separate bounds. Engine cleanup runs even when `on_unload` fails. |

The legacy [manifest](../../crates/skate-mods/src/schema.rs) has one `entry` and
API `2`; this path remains supported for local/peer sessions. See the resource
SDK migration section for conversion to server-selected resources. The
legacy [archive cache](../../crates/skate-mods/src/archive.rs) is an extraction
cache with temporary lifecycle, not the new persistent content store. Its
fingerprints must not be used as cross-platform security identities.

## Validation coverage

The linked validation report records actual executable/socket/filesystem checks for cold download, warm reconnect, full process restart,
cross-server reuse, changed/corrupt/interrupted content, confidential server-file
exclusion, dependency failure, stop/restart/disconnect cleanup, stale generations,
spoofed requests, scoped permissions/persistence, and Windows/Linux identities.
Headless tests do not establish graphical engine activation or live mixed-OS
gameplay. Transport-independent APIs also do not imply that a Steam dedicated
adapter has been implemented.
