# Native interiors and location markers (version 1)

The engine loads static interior models, composes their authored terrain into
native collision, renders markers and the minimap, owns the floor chooser, and
performs native travel. Lua supplies catalog data and optional marker settings.
The same service handles every interior; no apartment-specific host code is needed.

## Load a catalog

For an installed map, place a package at
`maps/locations/<lowercase map filename without .skate>/catalog.json`.
Both the game and dedicated server also accept `--locations PACKAGE_DIRECTORY`.
The catalog's `map` must match the active filename (or `test-world`). A missing
optional package has no effect; an explicit invalid package fails validation.

A local API 2 mod calls `sdk.locations.load("catalog.json")`. Paths are contained
within that mod package. Poll `sdk.locations.status()`; `ready` means model,
collision and destination checks completed, and `error` describes a failed load.
Catalog entries in `status.catalogs` include owner, generation, revision and
readiness. Loading is asynchronous. Replacements retain the previous valid
geometry on failure. Connected clients cannot load local collision catalogs.

See [the runnable two-floor example](examples/interior-catalog/) for a complete
synthetic GLB, matching shell, catalog and Lua wrapper. Enable it in `--test-world`;
its entry is at the origin. A single floor enters immediately; multiple floors
open the native chooser. Stop/unload returns occupants before releasing support.

Catalog coordinates are world-space metres, headings are radians, and matrices
are column-major affine transforms. Model and shell share the same transform.
`spawn`, entry, exit and `return_position` are floor anchors, checked against
native floor and standing clearance. Keep returns outside the entry radius plus
0.75 m. Place interiors away from existing terrain and one another.

## Dedicated resources

Set `"locations":"catalog.json"` in a resource manifest and list the catalog,
GLB and shell under `files`. Geometry is immutable admitted content: changing any
of it changes the resource identity and requires admission again. Clients prepare
matching terrain before becoming ready. Entry intents contain location/floor keys,
owner generation and revision; the host resolves coordinates, checks reach and
occupancy, and uses normal authoritative movement resets and return leases.
The native installed catalog is distributed by reserved owner `engine-locations`.

Script-owned catalogs require both `resource.locations` and `resource.teleport`
in the manifest and server grants. The reserved native provider is engine-owned.
Server Lua with an explicit `resource.locations` grant can publish:

```lua
resource.locations.set({generation="1",locations={{
  key="example-building", label="Apartments", enabled=true,
  style={color={1,0.75,0.15},opacity=0.3,radius=1,height=2}
}}})
```

Use the live owner's generation, never a cached prior generation.
`resource.locations.get()` reads its snapshot; `.clear()` restores catalog defaults.
Client resources use `sdk.locations.status()` with `engine.locations` permission.
Local mods use `sdk.locations.set(snapshot)` / `.clear()` for their own catalog.
Settings cannot change destinations, collision, another owner or safe return.
Disabled entries are rejected by the host as well as hidden locally.

## Bounds and assets

Version 1 permits 32 locations, 16 interiors and 8 floors per location; catalog
JSON is at most 128 KiB. A snapshot is at most 8 KiB, at most five publications
per second. Labels are 1–64 bytes; RGB/opacity are finite 0–1, radius 0.1–10 m,
height 0.1–20 m, and generation is a canonical positive decimal string.

Models follow existing resource import budgets: embedded static GLB, no external
paths/extensions, at most 16 MiB encoded, 2048 px image sides and 16 MiB aggregate
decoded texture references, and 16 MiB expanded geometry. The native renderer
supports base-color/emissive textures on TEXCOORD_0; bake normal, occlusion and
metallic/roughness maps before loading. Unsupported texture slots are rejected. Use `tools/prepare_interior.py` for the downloaded
apartment. Its independently authored shell is required; rendering never creates
collision. Shell JSON is `{version:1,positions:[[x,y,z],...],triangles:[[a,b,c],...]}`,
limited to 4 MiB, 65,536 vertices and 32,768 nondegenerate triangles. Retain creator
attribution and original source alongside the prepared package.

An optional snapshot entry `interaction="event-name"` asks the engine to notify
its owner when travel is requested. A local mod receives
`on_event({type="location",name="event-name",location=KEY,floor=KEY})`.
A server resource registers `resource.on("location", callback)` and receives
`{name,actor,exit,intent}` after an approved request. These notifications do not
replace native travel or grant access to another owner's catalog.
