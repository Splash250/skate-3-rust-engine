# Visual park authoring

Park Studio is a local browser editor over `tools/resource_park.py`'s placement
format and SKATE exporter. It runs offline with Python's standard library and a
modern browser. It does not connect to a public service or edit live servers.

```sh
python3 tools/park_editor.py --workspace resources --scene creator-park/placements.json --open
```

The editor prints a loopback URL with a one-session access token. Open that URL
if automatic browser opening is unavailable; reopen it after refreshing because
the token is kept only in page memory. The server listens only on `127.0.0.1` and
checks Host, Origin and the session token before file operations. It serves only
its fixed UI files. All scene/export paths are relative to `--workspace`;
absolute paths, traversal, symlinks and nonportable paths are rejected. Use a
scratch workspace when experimenting instead of changing a running server root.

## Edit and export

Choose an object from the object list or viewport. The object list, forms and
buttons work with a keyboard and expose native accessible labels. Use the
numeric inspector when pointer dragging is unsuitable.

- Top view shows X/Z; Front view shows X/Y; Isometric shows the park's shape.
- Drag selections in Top/Front view. Arrow keys move the selected object while
  the viewport has focus; Shift moves five steps. `Snap (m)` sets the grid step;
  zero disables snapping. Middle-drag pans and the wheel/slider zooms.
- Position, positive size and Y-axis rotation apply through **Apply changes**.
  Box/ramp size is metres; rail size scales local points. Rails have a fixed
  visible half-width of 0.06 m, independently of their native grind centerline.
- Rails use local X,Y,Z points, one per line. Add/edit/remove lines or use
  **Extend last point**. Open/closed polylines export native rail metadata.
- Markers select interaction, checkpoint or spawn-suggestion purpose, label,
  radius and checkpoint order. Markers render without collision. Marker purpose
  and order are exported metadata for server scripts; they do not grant rewards
  or teleport authority automatically.
- Duplicate/delete, Undo/Redo and Ctrl/Cmd+Z / Ctrl/Cmd+Shift+Z operate on complete
  validated revisions. History retains at most 64 revisions and 16 MiB per undo
  or redo list. Files are limited to 4 MiB, scenes to 2048 objects and editor rail
  points to 16384 total. Invalid edits leave the previous scene intact.
- Save/Load uses portable JSON. Load retains the previous scene in Undo history.
  Spawn position and heading are under **Park spawn**; heading is radians.

**Export resource** writes `placements.json`, `markers.json`, `park.skate` and
`resource.json`. Existing resource scripts, declarations, version, custom public
files and previously declared LODs are retained; the existing resource ID must
match. Exporting into a new directory produces a complete static-world resource.
The original [`creator-park`](../../resources/creator-park/README.md) example
adds server-observed marker guidance and private client UI. Its server reads
`markers.json` so edits cannot drift from hardcoded marker coordinates.

All coordinates are Y-up metres. Rotation is degrees around +Y; positive
rotation maps local +Z toward +X. The exporter shares one transform for static
render meshes, collision and native rail points. Save/export/reload round trips
are deterministic. JSON and `.skate` files contain no retail assets.

## Immediate native playtest

Provide your trusted built game and existing prepared asset directory:

```sh
python3 tools/park_editor.py --workspace resources --scene creator-park/placements.json --game-executable "$PWD/target/debug/skate3rust" --assets /absolute/path/to/prepared/assets --open
```

**Playtest** exports the current revision into a disposable temporary directory
and launches the actual native game with `--map` and `--assets`. There is one
playtest process at a time. Close the game or select **Stop playtest** before
launching a changed revision. Exiting the editor stops its playtest and cleans
its temporary export. The same gated process-group / Windows Job containment as
the local supervisor retires crash reporters and other descendants, even when
the game launcher exits first. Linux development launches configure the trusted
binary's sibling `deps` and installed Rust target libraries as `PLAY.sh` does.
Logs rotate at 1 MiB with one previous file; completed playtests retain only a
4 KiB diagnostic tail in the UI. Process status refreshes automatically without
overwriting inspector edits. Missing binaries/assets produce actionable errors.
No recipe/downloaded executable or browser-supplied command can be launched.

Direct map playtest exercises meshes, collision and native rail geometry; it
uses the local game and therefore does not execute dedicated resource scripts.
To test marker/gameplay behavior, export the scripted example, select it on a
local dedicated server and connect clients normally. World download, content
readiness, native installation and generation cleanup use the existing resource
pipeline. The graphical two-client scenario remains separate acceptance evidence
from editor interactions and deterministic encoding tests.

## Ownership and validation

Authoring state belongs to the editor's local workspace. A playtest uses an
immutable temporary export. Export itself publishes no live world change. The
server operator selects/restarts the exported resource, causing its normal
content revision/admission transition. Disconnect, replacement and unload retire
visuals, colliders and native rails through the existing world lifecycle; the
editor never registers live native handles or changes authoritative instances.

Run `python3 -m unittest tools.test_resource_park tools.test_park_editor -v` for
transform agreement, winding, marker separation, script-preserving export,
save/reload/history, HTTP session/origin rejection and filesystem confinement.
A fresh Chrome session additionally exercised ramp selection/rotation,
undo/redo, rail extension, save and a 21,064-byte actual export. The Linux
Playtest button loaded that edited scene in the native Vulkan game: 112 render
triangles, 88 collision triangles, one grind spline with four primitives, and
stable ground physics. Stop retired both native processes, the containment guard
and the temporary export. Exact local logs and the separate two-client gameplay
acceptance belong in the [evidence ledger](platform-extension-evidence.md).
This does not establish native Windows acceptance.
