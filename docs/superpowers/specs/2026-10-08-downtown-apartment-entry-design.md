# Downtown apartment and location markers

Status: written design approved by the user on 2026-10-08.
Date: 2026-10-08
Branch: proper-map

## Intent and agreed presentation

Install the downloaded Modern High-Rise Apartment (VR Scaled) by LoneDeveloper
as an enterable Downtown apartment. Connect it to a building entrance with an
Apartment pin in the existing live map and GTA V style world entry markers.
The user previously requested configurable RGB, an intersection-only top-left
A / PlayStation cross prompt, teleport actions, and a floor chooser when a
building has multiple destinations. Keep the original interior texture colors
and baked markings; do not add a hologram treatment.

The first installed building has one destination, named Modern Apartment.
The machinery accepts multiple floors, but this delivery does not invent or
duplicate apartments to populate a floor list. Existing multiplayer names and
YOU markers continue to work. Preserve all current working-tree changes.

## User flow

On Downtown, a house/apartment map pin identifies the entrance and its label.
A translucent yellow vertical entry cylinder and ground ring mark the same
world location. RGB, opacity, radius, height and label are data fields.
The prompt appears only when the local player's physical volume intersects the
entry volume. A on Xbox, cross on PlayStation and E on keyboard activate it.
Resolve overlapping markers deterministically by nearest interaction center.
Hide entry prompts during menus, loading, replay and expanded-map navigation.
A held button cannot repeatedly trigger travel or leak into a skating action.

For this single-floor building, activation starts entry directly. For multiple
floors, activation opens a list of destination names; choosing a floor starts
entry, and cancel leaves the player outside. The list uses existing UI input
ownership and releases it on close, map replacement or owner retirement.

Show loading feedback while preparing the interior. Teleport as soon as assets,
collision and destination are ready, with a ten-second readiness timeout rather
than a mandatory ten-second countdown. Failure keeps the player at the entrance
and displays an actionable message. Zero travel velocity, reset the native
camera/actor through the existing travel path and preserve the skater's identity.

An interior exit marker offers the same button prompt and returns to a validated
point outside the building, offset from the entry volume to avoid retriggering.
The apartment is a shared space: connected players in it can see one another.
While inside, the compact map uses the apartment geometry and bounds, shows YOU,
local peers and the Exit pin. Return restores the Downtown map view and entrance
pin; the interior must not expand the exterior map's overview bounds.

## Integration choices

Recommended: a reusable native location/interior service, with the apartment as
its first catalog entry. It owns loading, collision readiness, marker rendering,
interaction input and travel state. Local Lua mods and server Lua resources
configure locations through bounded host commands; the native map remains the
renderer. Existing Lua graphics, map, UI and server teleport facilities supply
integration patterns and transport. Native defaults also work in singleplayer.

An alternative is a local Lua mod using mesh buffers, map layers, colliders and
player teleport. It is smaller initially, but dedicated clients do not discover
standalone local mods and verified native-input attempts reject local physics
and teleport mutations. It cannot satisfy the multiplayer requirement by itself.

A second alternative is baking the apartment into a new Downtown map package.
That handles geometry as ordinary terrain, but couples this building to a
replacement world and makes programmable location ownership and cleanup harder.
Keep Downtown's original package untouched and use an additive interior catalog.

## Verified existing systems and required additions

Current code has:

- CurrentMap generation and transactional map replacement.
- Package-scoped GLB loading in modding/graphics.rs.
- Native player travel in modding/session.rs.
- Server resource.teleport approvals and host-owned return leases.
- MapLayerRegistry ownership and generation cleanup.
- Browser/canvas UI and resource-scoped event/state transport.

These facilities do not yet constitute an interior system. A GLB rendered by
sdk.graphics.mesh does not automatically become walkable native terrain.
Server teleport approval also does not install destination collision.
Add matching interior collision on client and authority before allowing entry.
Do not claim multiplayer support from a cosmetic model or a local teleport test.

## Native service and geometry lifetime

Keep location definitions separate from runtime state. Definitions identify the
active map, entrance, destinations, exit, model path, model transform, collision
path/transform, spawn position/heading and marker styles. Runtime records are
scoped by owner, owner generation and CurrentMap generation.

Interior preparation loads the visual GLB and a simplified collision shell off
the frame's critical path. Install floors, walls and major furniture collision;
do not feed the entire 421k-triangle visual model into collision. Use the same
world-space transforms for render, collision, destination and map geometry.
Retain the interior's complete render data before any streaming eviction.

Place the apartment in a reserved world-space pocket outside Downtown's terrain,
so its shell cannot intersect retail buildings. Normalize its source origin
(the GLB currently spans Y=17.52..20.04 metres) into the destination transform.
Choose and record an actual doorway and safe outside return point from a live
Downtown check; reject placements without ground/support or with blocked spawns.
Coordinates are catalog data, not a change to Downtown's map layout or format.

On map replacement, clear marker/input/UI state and retire all old-generation
interior geometry, collision and map handles. On mod/resource stop, return any
occupant safely before removing their supporting collision; use the existing
host travel lease lifecycle for dedicated resource-owned visits. Failed loads
never leave an actor above missing terrain.

## Multiplayer and programmable ownership

Singleplayer uses native travel. Dedicated multiplayer validates entry against
the authoritative location, actor position, current world/generation and ready
content; a client sends an entry intent identifying a location/floor, never an
arbitrary destination. Use existing resource event/state delivery and
resource.teleport approvals, without a new network packet format.

Interior collision must participate in the authority's ground/collision world
and the owning client's matching native terrain. Extend preparation/identity
handling for additive interior collision and validate matching content through
the existing world/resource admission identity mechanisms. Never suppress
verification or allow a client cosmetic model to redefine authoritative ground.
Do not rely on resource.entities primitive bodies being interchangeable with
native skating terrain; current entity support is a separate simulation path.

Server-configured locations use explicit grants, validated bounded definitions,
existing asset distribution and resource-owned state. Local mods can manage
only their own local locations. Defaults are native-owned; one resource cannot
remove another owner's locations. Proposed location API names and grants must
be documented as new contracts in the implementation plan and implemented
consistently across schema, runtime wrappers, host, declarations and examples.

Use bounded location/floor collections, finite coordinates, contained asset
paths, validated RGB/opacity/volume sizes and generation-aware asynchronous
completion. Invalid replacements preserve the previous valid configuration.
World rendering and travel approval are distinct operations.

## Asset delivery

Keep the downloaded original GLB and its creator/source attribution. Do not
restore the removed permission statement. The original has 11 meshes, 9 materials,
6 embedded textures and 420881 nondegenerate triangles, approximately 18.4 MB.
Khronos validation reported zero errors and warnings, but there has not yet been
an in-game validation.

Create a runtime variant appropriate to the actual import path. Dedicated
resource defaults currently permit a 16 MiB encoded model, 2048px image sides,
16 MiB aggregate decoded texture references and no glTF extensions. The source
uses 4096px atlases and KHR_materials_unlit. Fit the runtime variant into policy
through texture sizing/packing and an equivalent supported baked material
representation; do not silently raise client import limits. Preserve visual
appearance with a close-up comparison and retain the higher-resolution source.

Store large downloaded/private runtime assets in ignored local asset/package
storage. Check in reusable code, catalog/configuration and synthetic fixtures;
retain creator/source attribution with the asset. Do not commit or push.

## Scope boundaries

Deliver the apartment, exterior/interior map pins, native configurable cylinders
and ground rings, input prompts, entry/exit travel, and a multi-destination
chooser backed by the new location service. Mission markers may use the same
marker styles with script interaction callbacks, but this delivery does not
implement a mission/progression system. The previously requested broader HTML
map dashboard, event stars/completion and map-wide quick travel remain separate
work; the location service must leave those integration points available.

## Verification and acceptance

- Inspect source GLB structure and collision-shell geometry; check finite
  transforms, indices, safe spawn/support, material preservation and import budgets.
- Unit/integration tests cover volume intersection, nearest marker selection,
  button edges, input ownership, generation cleanup, timed load failure,
  safe return, invalid definitions and stale load completions.
- Run focused Rust checks/tests for changed game/mod/server interfaces and
  package validation for any delivered Lua package.
- Live Downtown check: entrance placement, map pin agreement, controller-style
  prompt, keyboard entry, walkable rooms, walls/floor collision, exit and return.
- Two-client dedicated check: server-approved entry, authority/collision match,
  shared interior player names/markers, one player's exit without moving another,
  reconnect, resource stop and map replacement. Check verified native skating
  separately if enabled; document actual mode exercised and unmet prerequisites.
- Capture exterior and interior screenshots from the running game, including the
  map, and compare runtime material appearance with the source preview.
- Run graphify update . after code changes and report actual results.
