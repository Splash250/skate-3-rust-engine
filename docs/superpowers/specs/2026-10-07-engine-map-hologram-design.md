# Built-in Live Map and Hologram Design

## Goal

Give players a live view of the currently loaded game world, with an always-on
small hologram and a larger, more detailed map they can open. The feature is
part of the game engine and works by default in stock singleplayer as well as
multiplayer.

## Behavior

- Show a compact, translucent, angled overhead view during gameplay. It follows
  the local skater and marks that position as **YOU**.
- Let the player open a larger, more detailed view of the same active map. Both
  views use the same current world and player state; they do not require
  map-specific layouts or server resources.
- Derive the map view from the geometry the engine has loaded. When a map
  transition commits, replace the map view and its bounds with the new world.
  The first version covers the active map, not a browsable catalog of installed
  maps.
- In multiplayer, show a marker and available player name for each connected
  remote skater. Remove markers when players leave. In singleplayer, show only
  **YOU**.
- Keep markers tied to world-space positions, including height, so transitions
  and elevated areas do not leave stale or incorrectly projected markers.
- Keep the map presentation client-side. Do not change the network protocol,
  dedicated-server authority, map file format, or resource API.
- If a world has no usable map geometry, keep gameplay running and show a clear
  unavailable state in the map view.

## Architecture

Add a native game map-view module and plugin. During map preparation, build a
bounded overview representation from the whole active map, including geometry
outside the gameplay renderer's current streaming radius. Preserve source UVs,
albedo textures, and material colors in that representation. A secondary 3D
camera renders it to a slightly transparent image used by the compact HUD and
expanded map UI. The map camera uses render filtering so it sees world geometry
without capturing the normal skater models or the map UI itself. The engine
obtains the map bounds from the active world and frames the overhead view from
those bounds; the small view follows the local skater while the expanded view
shows more of the world.

The map-view update reads the local `PlayerRoot` transform and the current
multiplayer actor transforms/identities. It projects those positions into the
map view and draws labels above the rendered map: **YOU** for the local player,
and each available remote name for other players. It updates remote markers
from the existing client replication state and never sends additional player
data over the network.

Map rendering state is associated with the committed map generation. A map
transition rebuilds the view bounds and render target only after the new scene
is published. Cleanup releases the previous map view assets and entities.

## Input and presentation

The compact map is present by default during gameplay. The expanded view opens
and closes with a native map action available to keyboard and controller
players. Keyboard `+`/`=` and `-` zoom around the local player; close zoom uses a
higher-detail representation of the active map. The controls preserve existing
pause/menu input behavior and keep player names and location markers readable.

## Failure handling and performance

Map-view setup and rebuild failures are isolated from gameplay. The affected
view reports that map data is unavailable, and a subsequent successful map
transition can rebuild it. Render-target resolution and update cost must be
bounded for the compact view; the expanded view may use a larger target while
open. Avoid retaining geometry or GPU assets from retired maps.

## Acceptance checks

- On a normal stock singleplayer launch, the compact map appears without
  enabling a mod or resource and shows **YOU** at the local skater's location.
- The expanded view opens from keyboard and controller input, is more detailed
  than the compact view, and closes without disrupting gameplay controls.
- The view frames more than one supported map using loaded geometry without
  per-map authored minimap data. It updates after a committed map change and
  releases the prior map's state.
- Two connected clients see **YOU** and the other player's moving marker/name;
  a disconnect removes the departed player's marker.
- The procedural test world and missing/invalid map geometry do not crash the
  game or block skating.
- Focused tests cover map-bound calculation, world-to-view projection, player
  marker lifecycle, and map-generation replacement. A live graphical check
  covers the overlay and both display sizes.
