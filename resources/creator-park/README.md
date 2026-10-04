# Creator Courtyard

An original, redistributable park authored with the placement pipeline: rotated
ramps, a ledge, a curved native grind rail and interaction/checkpoint markers.
All geometry and scripts are original project examples; no retail data is used.

Open its editable scene from the repository root:

```sh
python3 tools/park_editor.py --workspace resources --scene creator-park/placements.json --open
```

Set the export folder and resource ID to `creator-park`. Export preserves the
JavaScript scripts and grants. `server.js` reads the generated `markers.json`,
so moving a marker and re-exporting changes its observed gameplay location.
Marker entry publishes private practice guidance; it does not award verified
scores. This example's checkpoint order labels are guidance, not a race verifier.
Only the server selects spawn pads and detects entry from accepted observations.

Select `creator-park` in a server configuration and grant `resource.state`,
`resource.teleport` and `engine.ui`. Select only one required world. Start clients
with the normal dedicated `--test-world --connect HOST:PORT` workflow; world
admission, collision and native rails follow the resource pipeline.

To playtest directly, add `--game-executable /absolute/path/to/skate3rust --assets
/absolute/path/to/prepared/assets` to the editor command. The Playtest button
exports a temporary map and launches the actual game. Direct playtest exercises
local geometry and rails; server markers require a dedicated session.

See [visual authoring](../../docs/multiplayer/visual-authoring.md) for controls,
ownership and the distinction between local playtest and dedicated acceptance.
