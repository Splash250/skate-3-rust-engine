# Programmable Live 3D Map Design

**Status:** approved in chat on 2026-10-07
**Date:** 2026-10-07

## Purpose

Make the built-in live map useful on dense maps such as DownTown and expose it
as a supported extension surface for server Lua resources and local Lua mods.
The map remains a live-rendered 3D view of the active world. It keeps the
world's source colors, textures, and markings, stays lightly transparent in
the HUD, follows the local skater, and reveals more geometric and texture detail
as the player zooms in.

The Downtown capture showed that projecting a large fraction of every source
triangle into a single overview mesh makes dense geometry read as visual noise.
The renderer must preserve recognizable 3D structure while controlling that
noise and cost at overview scale.

## Goals

- Keep a small live map visible during gameplay and retain the larger expanded
  map and local-player marker.
- Render the active map from its loaded world geometry, including geometry
  outside the gameplay renderer's current streaming radius. Do not require
  authored per-map layouts.
- Keep source albedo textures, colors, and markings. Use a restrained,
  slightly transparent presentation without a hologram treatment.
- Increase visible detail as the player zooms: select progressively richer
  geometry and texture resolution, not merely enlarge the same coarse image.
- Make the map programmable by server Lua resources and local Lua mods through
  a stable, documented API.
- Let server resources publish synchronized map settings and overlay layers;
  let local mods add client-only layers.
- Keep gameplay, map transitions, and multiplayer player replication
  independent from map rendering failures.

## Non-goals

- Replacing the active world's terrain or source map geometry from server Lua.
- Changing gameplay collision, map streaming, or player authority to match the
  map representation.
- Adding a new network protocol, server asset requirement, map file format, or
  external engine fork.
- Running arbitrary Lua callbacks in the render loop. Scripts provide bounded
  data; native code renders it.

## User experience

The compact map is always visible in normal gameplay. It uses a stable,
north-up camera heading and a shallow overhead perspective so streets,
building forms, and elevation remain legible. The camera follows the local
skater while keeping a useful neighborhood context. The expanded map opens
with the existing map action; keyboard and controller zoom controls center on
the local skater. At each zoom threshold, the renderer selects a more detailed
map representation and higher target resolution. Zooming out returns to the
bounded overview representation.

The map keeps source textures and colors with subtle transparency. The
overview reduces small geometry and fine surface detail that collapse into
noise at HUD scale, while retaining large structures and street-level forms.
Close views restore those details. Depth testing and occlusion remain active
inside the map camera so the view reads as a small 3D scene rather than a
flattened pile of faces. Player markers and Lua layers render above the map
geometry for consistent visibility.

The local marker remains labeled `YOU`. Connected remote skaters use existing
replicated transforms and names. Server-provided map labels and markers remain
distinct from player identities.

## Lua programming model

Add a documented map namespace to the Lua SDK. It supports:

- Server-owned map settings, including layer visibility, presentation opacity,
  initial zoom policy, and map-specific text or legend entries.
- Server-owned overlay layers containing keyed markers, labels, paths, and
  bounded regions. Each item has a stable mod-owned key, world-space
  coordinates, and validated style fields such as color, opacity, and size.
- Replace, update, and remove operations by owner and key. One resource cannot
  overwrite or remove another resource's items.
- Client-local map layers from local mods, which are never published to other
  clients.
- Read-only queries for the active map identity, generation, bounds, player
  locations/names available to the client, current zoom, and expanded state.

The server resource owns canonical shared map state. Its map calls validate
finite coordinates, bounds, string and collection limits, supported style
values, and resource ownership before publishing a replacement state through
the existing server-owned resource state channel. Clients render that state
through native map layers. The implementation must use existing resource
transport budgets and generation/lifecycle checks; it must not send geometry
assets or textures through state updates. The first version accepts vector
map items only; it does not accept arbitrary image paths, meshes, or server
filesystem references.

Local mods can write only their own local layer collection. They cannot
replace server-owned state. Server state remains authoritative if a client
also has local map mods. The built-in terrain remains generated by the engine
from the active map; overlays do not alter collisions or world geometry.

The API declarations, capability discovery, examples, and SDK reference must
match runtime behavior. The API is reusable by any resource and must not
special-case a particular resource ID or game mode.

Server resources and local mods remain separate SDK surfaces. Server map
writers require a new `resource.map` grant. Local map writers require a new
`engine.map` capability. Map inspection requires the relevant map grant.
Update both manifest/capability documentation and Lua declarations; do not
silently grant map writes to every resource.

## Renderer architecture

Keep the map renderer client-side and separate from the gameplay camera. Build
map render data from the complete active source scene at preparation time,
before streaming discards nonresident render meshes. Associate every map
representation and layer snapshot with the committed map generation. On map
replacement, retire old geometry, targets, labels, input registrations, and
Lua-owned layers before displaying the new generation.

Replace the single quantized-mesh strategy with a map-specific level-of-detail
pipeline. The overview level should use a bounded screen-space representation
that retains broad building and street silhouettes and avoids spending its
triangle budget on tiny props or near-duplicate surfaces. Zoom levels should
progressively restore source triangles/materials and target resolution. The
detail budget must be selected per active map within explicit memory and frame
time limits; a single fixed cap must not silently discard an arbitrary portion
of a large map such as DownTown. Source UVs, texture references, and material
IDs must survive every LOD build. Geometry simplification must preserve
connected surface coverage and avoid visible snapped-face artifacts.

One dedicated camera renders only the active map LOD to a transparent target.
It excludes UI and player models. The native HUD composes the target,
player markers, player names, Lua layers, and optional legend. Only the
currently required LOD and target render each frame. Render targets, map
geometry, and overlay entities are released on map generation replacement or
resource retirement.

The HUD reconciles server and local layers by `(owner, layer, item key)`.
Replacing a resource state atomically updates that owner's complete snapshot;
removals and disconnects cannot leave stale markers. A layer update is
presentation-only and never mutates `PlayerRoot`, `NetworkActor`, map meshes,
or server authority state.

## Limits and failure behavior

The API caps each resource at 8 layers, 128 items, 512 total path/region
points, 64 UTF-8 bytes per label, and 5 published updates per second. Its
serialized state is limited to 8 KiB per resource update, below the existing
default resource-state payload budget. Invalid or over-budget updates return
an actionable Lua error and preserve the prior valid layer snapshot. One
resource's failure does not disable another resource's layers or the built-in
map.

If map geometry or an LOD is unavailable, keep gameplay running, report the map
as unavailable or show the nearest valid coarser LOD, and preserve Lua layers
that can still be projected using current bounds. A missing or stopped
resource retires its layers. A failed map transition leaves no old-generation
markers on the new world.

## Testing and acceptance

- Geometry and render tests cover Downtown-scale input, sparse distant areas,
  LOD transitions, preserved UV/material assignments, and bounded memory.
- Rendering checks show recognizable Downtown building/street structure at
  overview scale and increasing source detail at each zoom level, without the
  blocky triangle pile seen in the current capture.
- HUD checks cover compact/expanded presentation, transparency, local and
  remote markers, labels, and coexistence with script layers.
- Lua host tests cover ownership, server-only mutation, validation, state
  replacement, limits, failure retention, and cleanup on stop/restart.
- Multiplayer integration verifies server-authored layers arrive on all
  admitted clients, local-only layers remain private, and resource stop,
  disconnect, and map transition remove stale content.
- SDK declarations and examples demonstrate one server-authored route/region
  and one local-only marker layer. Document actual payload and item limits.
- Run applicable Rust checks, resource-runtime tests, the mod package checker,
  graphical Downtown zoom checks, and `graphify update .` after code changes.

## Supersession

This proposal extends the approved built-in map design. It supersedes only the
earlier restrictions that map presentation must be client-only and that no Lua
resource API may be added. It retains the client-side renderer, default HUD,
dynamic active-map geometry, multiplayer player markers, no map-format change,
and no new network protocol. Server Lua controls synchronized presentation
data through the existing resource-state transport; native clients remain
responsible for rendering.
