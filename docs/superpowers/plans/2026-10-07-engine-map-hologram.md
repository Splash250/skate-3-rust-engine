# Built-in Live Map Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans for native execution, or superpowers:subagent-driven-development if the user selects that method. Track the steps below as they complete.

**Goal:** Provide a default compact hologram and an expandable live map with local and multiplayer markers.

**Architecture:** Prepare a bounded, independent overview mesh from the entire active map before streaming removes gameplay meshes. Publish it with the scene, render it through a dedicated orthographic camera into a HUD image, and overlay projected player labels. Associate presentation state with `CurrentMap.generation` and retire geometry through existing scene ownership.

**Tech Stack:** Existing Rust and Bevy 0.18.1; no new dependencies.

**Spec:** [Approved design](../specs/2026-10-07-engine-map-hologram-design.md), approved in chat on 2026-10-07.

## Global Constraints

- Native client feature, enabled by default in stock singleplayer and multiplayer.
- No server resource, mod, protocol change, map-format change, or engine fork.
- Derive the overview from the whole active map, including nonresident streaming cells.
- Local label is `YOU`; remote labels use existing replicated identities and available names.
- Invalid overview geometry must not prevent gameplay or a subsequent successful rebuild.
- Work in the existing `proper-map` checkout; preserve unrelated work; do not commit or push.

## Review Focus

- Sparse distant geometry must survive overview reduction and affect framing (Task 1).
- Invalid indices, nonfinite coordinates, and zero-area geometry must not panic or yield invalid cameras (Task 1).
- A replacement map must release old overview assets and remove stale labels, including when its overview is unavailable (Tasks 2–3).
- Elevated players, wide/tall windows, and offscreen players must project and clip consistently (Tasks 2–3).
- Opening menus, replay, or losing focus must not trigger map input or leave a held controller button latched (Task 4).

## Decisions and source evidence

`PreparedScene::prepare` receives the full `SkateMap`; `prepare_resource` adds optional LOD packages. Build the overview once from the primary package, without duplicating LOD geometry. `streaming::install` removes mesh assets and `Mesh3d` components, so a query over resident gameplay meshes cannot implement whole-map coverage. Procedural geometry comes from `physics::ground::surfaces()` and needs the same overview path.

`MapAssets::retire` already owns scene mesh/material release and despawns `MapEntity`. Use that ownership for overview geometry. UI, camera, and render-target ownership remain in the map-view plugin, which checks committed generation after publication. Startup uses `world::spawn` and must work at generation zero.

`NetworkActor` and `PlayerRoot` expose presented transforms. Existing `Multiplayer::skater_name` supplies labels. Match existing nametag treatment of suspended peers; do not add replication state. Read current transforms through their ancestors in PostUpdate before UI layout; Bevy UI layout itself precedes transform propagation, so waiting for propagation here would create a schedule cycle.

Use render layer 30 (current native overlays use 28, 29, and 31). The dedicated camera must disable UI rendering and be excluded from gameplay camera queries. Preserve source texture UVs, albedo textures, and material colors; blend the map surface lightly over the transparent target. Keep the HUD panel dark and restrained.

Keyboard `M` without Ctrl/Alt/Super modifiers toggles the expanded view; controller D-pad Left toggles it during gameplay. Back/Select is already replay, Start is pause, and D-pad Right + R3 is debug camera. The expanded view stays live and does not pause or capture skating controls. Escape/Start keep their existing menu behavior and collapse the expanded map when a menu opens. Input is suppressed during replay, menus, loading, unfocused windows, and existing UI input capture.

## Task 1: Bounded whole-map geometry

**Files:** Create `crates/skate-game/src/map_view/geometry.rs` and its inline tests; declare the module through `map_view.rs` and `main.rs` as needed for tests.

**Interfaces:** `OverviewGeometry { positions: Vec<[f32; 3]>, bounds: OverviewBounds }`; `OverviewBounds { min: Vec3, max: Vec3 }`; `from_map(map: &SkateMap) -> Option<OverviewGeometry>`; `from_test_world() -> Option<OverviewGeometry>`. Positions form a triangle list in world coordinates.

- [x] Add tests for two distant clusters, a translated map, invalid indices/nonfinite vertices, degenerate triangles, empty geometry, and procedural surfaces. Assert finite bounds covering both clusters and `None` for unusable inputs.
- [x] Run `cargo test --locked -p skate-game --bin skate3rust map_view::geometry`; confirm the new assertions fail before implementation.
- [x] Implement bounds from all valid source triangles and deterministic spatial reduction: preserve original geometry up to 262,144 triangles; above that, quantize shared vertices onto a world-space grid with independent per-axis scales, deduplicate triangles, and progressively coarsen the grid until within the same cap. Keep a representative source triangle for a collapsed key so isolated small regions survive. Bound the deduplication set and output allocation, aborting an over-budget pass early. This supersedes centroid sampling after the University graphical check showed holes in continuous surfaces. Add a dense flat-map coverage regression and retain real 3D height in the overview.
- [x] Run the geometry suite, including an over-budget fixture asserting the cap, deterministic output, and coverage of a sparse distant cell.

## Task 2: Scene ownership, camera, and projection

**Files:** Create `crates/skate-game/src/map_view.rs` and `map_view/view.rs`; modify `map_render.rs`, `main.rs`, and `app.rs`.

**Interfaces:** `MapViewPlugin`; `stage_overview(geometry: Option<OverviewGeometry>, commands: &mut SceneCommands, meshes: &mut impl AssetSink<Mesh>, materials: &mut impl AssetSink<StandardMaterial>)`; `OverviewScene` component containing bounds; `ViewFrame` containing the camera transform and orthographic projection; `frame(bounds: OverviewBounds, focus: Vec3, expanded: bool, aspect: f32) -> ViewFrame`; `project(frame: &ViewFrame, point: Vec3) -> Option<Vec2>` returning normalized viewport coordinates.

- [x] Add tests for translated/flat bounds, all eight expanded-view bound corners inside the viewport, elevated markers, offscreen clipping, and non-square aspect ratios. Run `cargo test --locked -p skate-game --bin skate3rust map_view::view` and confirm failing assertions before implementing.
- [x] Stage one overview during normal/procedural preparation and only the primary package during resource preparation. Spawn with `MapEntity`, layer 30, no streaming `Cell`, no shadows, and existing staged mesh/material ownership. Failure yields unavailable overview metadata without failing scene preparation.
- [x] Implement an angled, fixed-heading orthographic frame. Compact mode follows the player across a 120-metre span; expanded mode fits all eight bound corners with 10% padding. Compute depth range from bounds and reject nonfinite projection input.
- [x] Register the plugin in the shared client app. Create a dedicated image camera with UI rendering disabled and transparent clear color, ordered before the main presentation camera. Use a 320×240 compact target and a 1280×960 expanded target. Render only the active view; deactivate while hidden. Reallocate only on size or generation changes and explicitly remove retired image assets.
- [x] Add a synthetic ECS test that publishes an overview, simulates gameplay mesh eviction, and verifies the overview remains. Replace it twice, including an unavailable map; assert old meshes/materials/images/entities are removed and generation-zero startup works. Run `cargo test --locked -p skate-game --bin skate3rust map_view`.

## Task 3: Hologram HUD and player labels

**Files:** Create `crates/skate-game/src/map_view/hud.rs` and `map_view/markers.rs`; integrate through `map_view.rs`.

**Interfaces:** `PlayerMarker { id: Option<u64>, position: Vec3, label: String }` (`None` identifies local); `project_markers(frame: &ViewFrame, players: &[PlayerMarker]) -> Vec<(Option<u64>, Vec2, String)>`. HUD systems use a keyed entity set and update/remove labels from this desired list.

- [x] Add lifecycle tests: local-only `YOU`, remote join, movement, rename, disconnect, empty snapshot, and map replacement. Add height/aspect/clipping assertions using Task 2 projection. Run `cargo test --locked -p skate-game --bin skate3rust map_view::markers` to observe failures.
- [x] Build a top-right compact panel (240×180 logical pixels, scaled down on small windows) and a centered expanded panel fitting within 90% of the window at 4:3. Use the existing default UI camera explicitly. Show map name, toggle hint, local/remote dots and readable labels; clip contents within the map rectangle. Show `Map unavailable` when no overview exists.
- [x] Gather presented local and remote transforms every frame, using existing multiplayer activity/name APIs and suspended-peer rules. Project through the exact camera frame used for the target. Reconcile labels by identity; render dots as HUD elements so terrain cannot hide them. Place `YOU` first and shift nearby names vertically to avoid overlap without moving their projected dots.
- [x] Run marker and full map-view tests. Verify label entities remain bounded across repeated joins/leaves and mode switches.

## Task 4: Input integration and end-to-end checks

**Files:** Create `crates/skate-game/src/map_view/input.rs`; modify `map_view.rs` and `docs/LINUX.md` for control documentation. Inspect any newly needed scoped guides before edits.

**Interfaces:** `MapViewState { expanded: bool, generation: Option<u64>, previous_buttons: u16, zoom_level: u8 }`; pure `toggle_requested(keys: &ButtonInput<KeyCode>, buttons: u16, previous_buttons: u16) -> bool`. The input system runs in PostUpdate after controller sampling, menu/replay interaction, and map transition processing, before view preparation. ControllerFrame retains the physical snapshot after gameplay consumes its packets.

- [x] Add tests for `M`, modified `M` suppression, D-pad Left rising edge, held-button suppression, menu/focus/replay suppression, generation reset, and release after a blocked interval. Run `cargo test --locked -p skate-game --bin skate3rust map_view::input` to confirm the regression tests fail first.
- [x] Implement the chosen controls and close/hide rules. Inspect current modal and UI-capture consumers before wiring gates; reuse existing read-only state instead of adding a second input backend. Update previous button state even while blocked. Keep pause/menu handling and gameplay scheduling intact.
- [x] Run `cargo test --locked -p skate-game --bin skate3rust map_view` and confirm a nonzero test count. Run `cargo check --locked -p skate-game --bin skate3rust --bin skate3-multiplayer`. Run relevant existing map-transition tests after listing their exact filter with `cargo test --locked -p skate-game --bin skate3rust -- --list`.
- [x] Run scoped `rustfmt --edition 2024 --check` on new files and inspect formatting/diffs for modified files. Check `git diff --check`.
- [ ] If the graphical session and owned assets are available, launch the client using existing Linux instructions: inspect compact/expanded views, move and change elevation, resize, use keyboard/controller controls, open menus/replay, transition between two maps, and check distant geometry. Connect two local clients through existing multiplayer facilities, verify moving labels, and disconnect one. If prerequisites are unavailable, report each unverified graphical/controller/multiplayer check explicitly.
- [x] Run `graphify update .` after code changes. Inspect the final diff and working tree; report actual test/build/graph results and remaining visual limitations. Do not commit or push.

## Plan review

Self-review: all spec sections map to the four tasks; geometry, projection, asset retirement, marker lifecycle, and input conflicts have explicit checks. Live GPU appearance, real controller behavior, and two-client behavior require runtime validation in addition to synthetic tests. The bounded overview intentionally omits some dense geometric detail; if sparse sampling makes surfaces unreadable in live checks, revise reduction before accepting the feature.

## Execution and verification record

Implemented on `proper-map` with spec and execution approval recorded in chat.
The game suite passes: **547 passed, 0 failed, 182 ignored**. This includes
whole-map reduction, dense surface coverage, separate elevations, projection,
marker reconciliation, HUD retirement and the real controller packet-consumer
regression. The sparse-region test permits half a reduction grid cell of
coordinate movement while requiring a complete nondegenerate distant face.

Repaired four pre-existing test compilation blockers (a missing closing brace
and missing fixture fields) to run the suite; existing assertions were preserved.
Independent review caught the consumed controller sample and verified its fix.

Graphical checks used isolated X11 displays and the existing GPU: procedural
world, BlackBoxPark compact and expanded views, and University. Two connected
local clients showed local/remote labels; host telemetry and capture confirmed
removal after peer disconnect. Six owned-asset preparation/commit/retirement
cycles passed for BlackBoxPark, University and the procedural world.

Physical controller operation, live resize, live replay/menu interactions and
live movement/elevation scenarios have not been manually exercised. Their
synthetic coverage does not replace those runtime checks. The graphical checklist
above remains partly open for that reason. Roofs remain visible as ordinary
world geometry; no authored floor/roof filtering is introduced.

Final client build and `cargo check` for both client binaries passed. Scoped
formatting and diff whitespace checks passed. `graphify update .` completed
with 36,888 nodes and 80,367 edges; it reported version, zero-node source and
community-label warnings. The revised University compact GPU capture shows
continuous ground after replacing face sampling. No commits or pushes were made.

Final University expanded GPU capture passed: keyboard M opens the centered whole-map view with YOU correctly projected. The harness needed a delay between acquiring X11 focus and injecting the key; initial same-frame focus/key input did not toggle.

Zoom-detail follow-up: the compact 262,144-triangle overview remains active at map scale. The expanded map accepts `+`/`=` and `-`; zoom centers on YOU and switches to a separate, initially hidden detail mesh after two steps. The detail budget is 1,700,000 triangles, enough for all 1,645,617 source triangles in University. The detailed mesh reuses the source textures/materials and stays map-owned for retirement.

Verified this follow-up with a successful `cargo build --locked -p skate-game --bin skate3rust`, `git diff --check`, and live University captures at map scale and after zoom. The close capture shows roads and building markings rendered from the source geometry. No test suite was run for this follow-up. `graphify update .` completed (36,900 nodes, 80,407 edges); existing graphify version and zero-node source warnings remain.

Multiplayer follow-up check: two direct-local clients loaded University, each reported one remote peer, and both expanded detail captures displayed YOU and the other Player marker/name. No code changes were needed.
