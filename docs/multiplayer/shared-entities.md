# Dedicated shared objects

Server resources with the requested **and granted** `resource.entities`
capability can call `resource.entity(command)` (also
`resource.entities.command(command)`). Lua, JavaScript and the managed host
route the same bounded JSON command to the server. Resource identity and
generation come from the host, never from command fields.

```lua
resource.entity({
  op = "spawn", key = "crate", instance = 0,
  shape = { type = "box", half_extents = { 0.5, 0.5, 0.5 } },
  body_type = "dynamic", mass = 5, position = { 1, 2, 3 }
})
resource.entity({ op = "impulse", key = "crate", impulse = { 10, 0, 0 } })
```

Shapes are `box` (`half_extents`), `sphere` (`radius`), and `capsule`
(`half_height`, `radius`). Body types are `dynamic`, `kinematic`, and `static`.
Spawn also accepts normalized xyzw `rotation`, `velocity`, `friction`, RGBA
`color`, and optional `controller`. Defaults are dynamic, mass 1, friction 0.7,
identity rotation, zero velocity, and an opaque blue material.

| Operation | Required fields | Effect |
| --- | --- | --- |
| `spawn` | `key`, `shape`, `position` | Allocate a new stable serial in the calling resource generation. |
| `remove` | `key` | Delete this resource's object and retire client replicas. |
| `impulse` | `key`, `impulse` | Apply a server physics impulse to a dynamic body. |
| `velocity` | `key`, `velocity` | Set dynamic or kinematic velocity. |
| `pose` | `key`, `position`, `rotation` | Relocate, clear velocity, and reset interpolation epoch. |
| `transfer` | `key`, optional `controller` | Assign an admitted player in the object's instance, or return the lease to the server. |

`controller` is a gameplay lease. Simulation and transform publication remain
server-owned. A resource may interpret authenticated player events to decide
which commands to issue; giving a player a lease does not grant transform
updates. A controller leaving or changing instance returns the lease to the
server. Stopping or restarting the resource removes its objects. Reusing a key
after deletion allocates a different serial. Controller and instance inputs
accept canonical decimal strings; use strings for values above JavaScript's
safe integer range. Host-injected generations and wire serials retain full u64
precision.

Each instance has a separate Rapier world. Replication uses the admitted
recipient's movement epoch and instance, plus a 500 m distance interest range
when a current player position is available. A fragmented inventory retires
objects that leave interest. Stable inventory revisions and repeated snapshots
recover loss and initialize late joiners; obsolete inventory, sample, resource
generation, and movement epochs cannot restore retired objects. Clients cannot
submit shared-object packets to the authority or impersonate its admitted
endpoint.

The client builds primitive visuals and finite-mass native collider shadows.
Position and rotation interpolate over an 80 ms buffer with at most 100 ms of
extrapolation. Host pose epochs reset that history. Objects with no fresh sample
for one second stop rendering and contributing collision. Instance changes,
disconnects and resource retirement remove native bodies, visuals and their
mesh/material assets. Walking and board queries include the shared solids.

Current bounds are 256 objects across 64 active instances, 64-byte resource and
object labels, 4 KiB script commands, and an independent 24,000 B/s per-recipient
entity lane with a 2,400-byte burst cap. Every datagram stays within the 1,200-byte
network MTU. Inventories contain at most eight 32-ID pages. These entity bounds
are currently fixed; general resource budget negotiation does not configure
them. Replication checks preserve body/pose movement priority.

## Verification and limits

`cargo test --locked -p skate-server --test entities` exercises two actual
loopback UDP clients observing a server-simulated body pushed by player contact,
a late join receiving its moved position, controller disconnect recovery and
resource restart deletion. Server unit tests exercise static/dynamic contact,
instance-separated physics and stale/cross-resource command rejection. Network
tests cover loss, obsolete generations/epochs, forged endpoints, bounded
inventory assembly and deletion. The native adapter is compile checked; these
tests do not establish graphical two-player acceptance.

Player contact proxies are upright capsules derived from recent, plausibility-
checked owner observations. They are not an independent simulation of the
recovered skateboard or articulated skater. Native clients retain their own
collision response and reconcile the object shadow against server snapshots.
Thus object-object simulation is server-owned, while player motion, tricks and
scores remain subject to the separately documented authority limits. This
implementation does not verify competition results.

When a required resource world is selected, the dedicated host validates its
embedded triangle collision and installs the same terrain in each instance.
Terrain replacement stages complete worlds, preserves object identity and
motion, and changes pose epochs so client interpolation retires old samples.
Without a required resource world, resources can supply supported static
objects; arbitrary private retail collision archives are not loaded by this
path. Shared model/convex authoring, cross-resource ownership transfer and
configurable entity budgets remain outside this primitive entity contract.
Graphical two-client acceptance is recorded separately from these tests.

For physical-interaction diagnostics, start the server with
`SKATE_RESOURCE_DIAGNOSTICS=1`. The first tracked solver contact between a shared
entity and a player proxy logs `RESOURCE_ENTITY_CONTACT` with `entity`, `actor`,
`resource`, `generation`, `key`, `instance` and `tick` fields. These records
exclude sensor overlaps and inactive proximity candidates. They measure the
server's coarse player proxy touching an object, not articulated client physics.
`Host::entity_contact_metrics()` reports contact-step counts and first/last ticks
for at most 256 entity/player pairs, plus an omitted-contact-step count when full.
The host prunes retired entities and departed or other-instance players each
tick and drains underlying physics contact events so historical events cannot
accumulate indefinitely. Diagnostics do not apply forces or change collisions.

## Scoped resource state and events

The default scope remains `{"kind":"resource"}`. Server scripts may select
`{"kind":"instance","id":"7"}`, `{"kind":"player","id":"42"}`, or
`{"kind":"entity","id":"99"}` with the optional scope argument to
`resource.state.set(key, value, scope)` and
`resource.send(event, value, recipient, scope)`. Scope IDs are canonical decimal
strings; numeric IDs are accepted only through JavaScript's exact integer range.
Each `(resource, scope, key)` is independent. The resource's existing state key
and byte quotas cover all its scopes together.

Player scopes reach only that admitted player. Instance scopes reach only
players in that instance. Entity scopes require an existing entity owned by the
sending resource generation and follow the entity's instance and 500 m interest
range. Entering interest initializes current canonical state; leaving interest
or deleting the entity removes it from the client VM. Departed player and entity
state is pruned on the server without discarding state for currently empty
instances. Clients cannot publish private-scope network events or authoritative
state.

An instance transition rotates the resource activation epoch as well as the
movement epoch. The host retires pending small and bulk messages, pauses resource
admission, and sends a fresh offer. The client retires the old VM/channel and
rejects records below the movement reset's epoch floor before receiving data.
Only visible canonical states are replayed after activation. This prevents
in-flight private room data from entering a later room, including when an old
packet arrives before the fresh offer.
