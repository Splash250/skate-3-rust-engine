# Programmable Live 3D Map Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Improve the built-in live 3D map so zoom reveals more useful source detail, and expose bounded server-authored and client-local Lua map layers.

**Architecture:** Keep map geometry and rendering client-side. Replace the current single quantized overview/detail pair with a progressive, material-aware LOD set built from the complete active map. Store server layers in one reserved server-owned resource-state value per resource; render them alongside separately owned local-mod layers in the native HUD.

**Tech Stack:** Rust 2024, Bevy 0.18.1, serde/serde_json, existing Lua VM and server resource-state transport.

**Spec:** [Programmable Live 3D Map Design](../specs/2026-10-07-programmable-live-map-design.md)

## Global Constraints

- Keep the compact map visible in gameplay and preserve the expanded map and local/remote player markers.
- Build from the whole active map before streaming eviction; do not add map-specific authored layouts.
- Preserve source UVs, textures, and material colors; render a restrained slightly transparent 3D view.
- Zoom must select richer geometry and texture resolution, not only enlarge the same image.
- Server resource map writes require the `resource.map` grant; local mod map writes require the `engine.map` capability.
- Limits: 8 layers, 128 items, 512 total path/region points, 64 UTF-8 bytes per label, 5 published updates per second, and 8 KiB serialized state per resource update.
- Use the existing resource-state channel; add no network protocol, map format, server asset requirement, or engine fork.
- Invalid updates preserve the owner's previous valid snapshot. Each resource owns only its own layers; local layers remain client-only.
- Preserve existing uncommitted work. Do not commit, push, or deploy.

## Review Focus

- **Downtown-scale source geometry:** At detail zoom, retain all source triangles when the map is within the 2,000,000-triangle detail ceiling; test a synthetic 1,888,894-triangle fixture for full triangle, UV, and material retention.
- **Dense overview geometry:** Small projected clutter must be reduced without losing large structures, continuous streets, or sparse distant regions; test a synthetic city block containing each class.
- **Malformed/oversized Lua layer state:** NaN coordinates, malformed styles, excessive counts, and serialized state over 8 KiB must be rejected while the old valid snapshot stays visible; cover validation tests.
- **Unauthorized or spoofed ownership:** A client cannot publish server layers and one resource cannot remove another resource's layer; cover host and game-host tests.
- **Generation and namespace cleanup:** Resource restart, disconnect, map transition, and a same-key local layer must not retain or overwrite unrelated content; cover lifecycle tests.

## File Map

- `crates/skate-game/src/map_view/geometry.rs`: whole-map geometry extraction, progressive LOD building, source UV/material preservation, and geometry tests.
- `crates/skate-game/src/map_view.rs` and `map_view/hud.rs`: LOD selection, target sizing, layer registry composition, and generation cleanup.
- `crates/skate-mods/src/map.rs`: shared serializable map settings/layer/item types, validation, and limits.
- `crates/skate-mods/src/resources.rs` and `resource_api.lua`: server resource `resource.map` namespace and reserved state publication.
- `crates/skate-mods/src/api.lua`, `vm.rs`, and `lib.rs`: client-local `sdk.map` wrappers, typed commands, and shared type exports.
- `crates/skate-game/src/modding/resources.rs`: ingest server-owned map state from the client resource host and retire stale snapshots.
- `crates/skate-game/src/modding/mod.rs`: apply local map-layer commands under the calling mod's ownership.
- `crates/skate-game/src/modding/engine_access.rs`: add the read-only `map_view` observation consumed by `sdk.map.status()`.
- `crates/skate-game/src/map_view.rs`: expose the client-side map-layer registry and read-only map status to Lua hosts.
- `crates/skate-mods/src/resources.rs` capability validation, `crates/skate-game/src/modding/resources.rs` client grants, and `crates/skate-server/src/resources.rs` server grants: authorize `resource.map` and `engine.map`.
- `sdk/resource.lua`, `sdk/skate.lua`, `sdk/RESOURCES.md`, `sdk/GENERAL_API.md`, `sdk/ENGINE_API.md`: declarations and references for both SDK surfaces.
- `resources/programmable-map/`: server resource example publishing a route/region.
- `sdk/examples/programmable-map/`: client-local mod example demonstrating a private marker layer.

## Task 1: Progressive 3D map geometry

**Files:** Modify `crates/skate-game/src/map_view/geometry.rs`, `map_view.rs`, and `map_view/hud.rs`.

**Interfaces:**

- Produce `MapLodSet { bounds: OverviewBounds, overview: OverviewGeometry, intermediate: OverviewGeometry, detail: OverviewGeometry }` from the full source map.
- Add `build_lods(map: &SkateMap) -> Option<MapLodSet>` and use `from_test_world()` as the procedural single-level fallback.
- Use triangle ceilings of 262,144 for overview, 750,000 for intermediate, and 2,000,000 for detail. If source geometry fits a level's ceiling, preserve it without reduction. For a larger map, reduce deterministically while retaining source UVs and material IDs.
- Select overview in compact mode, intermediate at expanded zoom level 1, and detail at zoom level 2 or above. Expanded render-target dimensions rise from 1280×960 to 1920×1440 at detail zoom.

- [x] **Step 1: Add failing geometry/LOD tests.** Add tests named `detail_lod_preserves_all_sub_two_million_triangles`, `lods_are_monotonic_and_keep_uv_material_pairs`, `overview_preserves_large_surfaces_and_sparse_regions`, and `lod_builder_rejects_invalid_source_geometry`. Use synthetic source triangles and assert exact source counts/UV/material mapping for detail, monotonically nondecreasing triangle counts across LODs, finite complete bounds, and deterministic output.
- [x] **Step 2: Run the map-view geometry filter and confirm the new expectations fail.** Run `cargo test --locked -p skate-game --bin skate3rust map_view::geometry`.
- [x] **Step 3: Implement `build_lods` and deterministic bounded reduction.** Partition triangles into world-space clusters; preserve material buckets and source UV triplets; spend coarse-level budgets on projected large surfaces first, then representative geometry per cluster so a distant block cannot disappear. Reuse source mesh/material handles for the detail LOD.
- [x] **Step 4: Wire the LOD set to zoom state.** Hide inactive LOD entities rather than rendering all levels together. Change the expanded target only when crossing the detail threshold, and release the old image asset.
- [x] **Step 5: Run the geometry and map-view suites.** Run `cargo test --locked -p skate-game --bin skate3rust map_view` and verify nonzero passing tests.

## Task 2: Shared map state schema and validation

**Files:** Create `crates/skate-mods/src/map.rs`; modify `crates/skate-mods/src/lib.rs`.

**Interfaces:**

- `MapSnapshot { settings: MapSettings, layers: Vec<MapLayer> }`.
- `MapSettings { enabled: Option<bool>, opacity: Option<f32>, initial_zoom: Option<u8>, title: Option<String> }`.
- `MapLayer { key: String, visible: bool, items: Vec<MapItem> }`.
- `MapItem`: tagged enum `Marker { key, position, label, style }`, `Label { key, position, text, style }`, `Path { key, points, style }`, and `Region { key, points, style }`.
- `MapStyle { color: [f32; 4], size: f32 }` and `validate_snapshot(snapshot: &MapSnapshot) -> Result<(), String>`.
- Define the limits in one place: 8 layers, 128 items total, 512 path/region points total, 64 UTF-8 bytes per label/title/key, world coordinates within ±100,000 m, RGBA channels in `[0, 1]`, marker/line size in `(0, 128]` logical pixels, opacity in `[0, 1]`, initial zoom in `0..=2`, and serialized owner state ≤8 KiB. Paths need at least 2 points; regions need at least 3.

- [x] **Step 1: Add failing schema tests.** Cover a valid mixed snapshot, duplicate layer/item keys, NaN/Infinity in coordinates or styles, alpha/size range failures, each count limit boundary, multibyte labels counted by UTF-8 bytes, and serialized size over 8 KiB.
- [x] **Step 2: Run the new `skate-mods` map schema filter and confirm failures.** Run `cargo test --locked -p skate-mods map::`.
- [x] **Step 3: Implement the serializable types and `validate_snapshot`.** Require layer/item keys of 1–64 UTF-8 bytes, world coordinates within ±100,000 m, RGBA channels in `[0,1]`, marker/line size in `(0,128]` logical pixels, opacity in `[0,1]`, initial zoom in `0..=2`, paths of at least 2 points, and regions of at least 3 points. Use deterministic ordered collections where snapshot ordering matters; reject invalid data without modifying the caller's previous snapshot.
- [x] **Step 4: Run the schema tests and existing crate tests.** Run `cargo test --locked -p skate-mods map::` then `cargo test --locked -p skate-mods`.

## Task 3: Server Lua resource map API

**Files:** Modify `crates/skate-mods/src/resources.rs`, `resource_api.lua`, `resources.rs` capability validation, `crates/skate-server/src/resources.rs`, `sdk/resource.lua`, `docs/multiplayer/resources.md`, and resource SDK tests.

**Interfaces:**

- `resource.map.set(snapshot)` validates a complete `MapSnapshot` and publishes it at the reserved key `__map_v1` for the current resource's default resource scope.
- `resource.map.clear()` deletes only this resource's `__map_v1` value.
- Both functions require `resource.map`; writes also use the existing server-owned resource state output. Client-side calls fail with a server-only error.
- Add `resource.map` to supported resource grants. Do not permit the reserved key through generic `resource.state.set`.
- Rate-limit successful publications to 5 per second per resource in `Shared`; reject an over-rate update without changing stored state or emitting a network output.

- [x] **Step 1: Add failing server-resource API tests.** Verify server `set` emits a validated `Output::State`, `clear` emits deletion, clients are rejected, missing capability is rejected, generic writes cannot spoof `__map_v1`, and invalid replacement leaves the previous state intact.
- [x] **Step 2: Run the focused resource API test filter and confirm failures.** Run `cargo test --locked -p skate-mods resource_map`.
- [x] **Step 3: Implement the Lua namespace, schema conversion, reserved-key handling, capability check, and resource grant validation.** Keep the existing network/state channel and its configured budgets.
- [x] **Step 4: Run resource API and server grant tests.** Run `cargo test --locked -p skate-mods resource_map` and `cargo test --locked -p skate-server resource_map`.

## Task 4: Client Lua API and owned layer registry

**Files:** Modify `crates/skate-mods/src/api.lua`, `vm.rs`, `lib.rs`, `crates/skate-game/src/modding/mod.rs`, `modding/resources.rs`, and `map_view.rs`.

**Interfaces:**

- Add typed local commands `MapSnapshotSet { snapshot: MapSnapshot }` and `MapSnapshotClear {}` to `Command` and parse `sdk.map.set(snapshot)`, `sdk.map.clear()`, `sdk.map.status()`.
- Add Bevy `MapLayerRegistry`, keyed by `(owner_id, source, layer_key)`, with `set_server(resource_id, generation, snapshot)`, `set_local(mod_id, snapshot)`, `remove_owner(owner_id, source)`, and `status() -> MapStatus`.
- `MapStatus` returns the current map name/generation/bounds, client-visible player ids/names/positions, zoom level, and expanded state. Publish it through the `map_view` engine observation and have `sdk.map.status()` read that snapshot synchronously. Reject client writes to server-owned entries.
- In `modding/resources.rs`, pull only `__map_v1` values from `Host::scoped_states()`, verify the live resource generation and the active `resource.map` grant, validate the snapshot, then replace that resource's registry entry. Retain no stale entry if the host drops the resource scope.
- Add `engine.map` to supported client capabilities and require it for local write commands and map queries.

- [x] **Step 1: Add failing SDK/host tests.** Cover local `set/clear/status`, missing capability, client attempt to create a server owner, server state ingestion with valid and stale generations, same layer/item key across different owners, and local/server namespace collision.
- [x] **Step 2: Run focused `skate-mods` and `skate-game` filters and confirm the new tests fail.** Run `cargo test --locked -p skate-mods map_command` and `cargo test --locked -p skate-game --bin skate3rust map_view::server_layers`.
- [x] **Step 3: Implement command parsing, owner-scoped registry reconciliation, resource-host ingestion, grant checks, and `MapStatus`.** Preserve the previous valid resource snapshot when validation fails; remove entries on generation change, resource retirement, disconnect, or map-generation replacement.
- [x] **Step 4: Compose registry snapshots in the HUD.** Render server and local layers above the map target, retaining their owner namespace and style; reconcile item entities by stable key and despawn removed items.
- [x] **Step 5: Run focused integration tests.** Run `cargo test --locked -p skate-mods map_command` and `cargo test --locked -p skate-game --bin skate3rust map_view`.

## Task 5: SDK declarations, examples, and end-to-end checks

**Files:** Modify `sdk/resource.lua`, `sdk/skate.lua`, `sdk/RESOURCES.md`, `sdk/GENERAL_API.md`, `sdk/ENGINE_API.md`; create `resources/programmable-map/resource.json` and `server.lua`; create `sdk/examples/programmable-map/mod.json` and `main.lua`.

- [x] **Step 1: Add Lua declaration tests or package fixtures for both API surfaces.** Ensure the server example can publish a styled route and region, and the client example can add/remove a private marker without claiming server ownership.
- [x] **Step 2: Run the local mod package checker and resource validation tests.** Run `cargo run --locked -p skate-mods --example check_mod -- sdk/examples/programmable-map`, `cargo test --locked -p skate-mods`, and `cargo test --locked -p skate-resources`.
- [x] **Step 3: Document side, capability, lifecycle, ownership, exact limits, world-coordinate rules, and error behavior.** State that v1 accepts vector items only; image/mesh/server-path references are rejected.
- [x] **Step 4: Run client builds and focused engine tests.** Run `cargo check --locked -p skate-game --bin skate3rust --bin skate3-multiplayer` and `cargo test --locked -p skate-game --bin skate3rust map_view`.
- [x] **Step 5: Exercise Downtown graphically.** Launch the owned-asset Downtown map, capture compact/expanded views at overview, intermediate, and detail zoom, and verify recognisable buildings/streets, source texture/color retention, visible transparency, and LOD changes. Run the server Lua example with two clients; verify its route/region arrives on both clients while the client-only marker remains local. Restart the resource, disconnect a peer, and transition maps to verify cleanup.
- [x] **Step 6: Finish repository verification.** Run scoped Rust formatting checks on changed Rust files, `git diff --check`, `graphify update .`, and inspect `git status --short`. Report the exact tests/builds/GPU/multiplayer scenarios exercised and any unavailable prerequisites. Do not commit or push.

## Plan Review

The spec's base-map, texture/transparency, progressive zoom, full-map coverage,
server/client ownership, capability, serialization, cleanup, documentation, and
multiplayer acceptance requirements map to Tasks 1–5. The five review-focus
risks each have a task-owned test above. The shared schema precedes both Lua
surfaces; the registry then consumes that schema; final examples and runtime
checks depend on all prior interfaces, so implementation should be sequential.


## Execution evidence (2026-10-07)

Implemented in the existing `proper-map` checkout without commits. The user's
subsequent Skyrim-style revision adds a perspective miniature, lit source
textures, map-only sunlight/shadows, and expanded-map rotation/tilt controls.
M opens; wheel or +/- zooms; RMB drag or Home/End and PageUp/PageDown orbit.

Focused map tests: 27 passed. Both client binary checks and the `skate3rust`
build passed. Shared schema tests: 2 passed. Resource-runtime tests and the
actual two-client server UDP/HTTP map publication test passed; resource package
validation and the standalone example checker passed. The broad skate-mods
suite stopped at the pre-existing missing Skyline GLB fixture. No full-workspace
or release build was claimed.

Downtown Vulkan acceptance used two clients and a local dedicated server:
compact, whole-map, intermediate/detail, orbit and wheel zoom; YOU and peer;
server route/region on both clients; private K marker on only one; resource
stop cleared public/private layers, restart restored public content; peer
exit removed its marker. Generation replacement and streaming retirement were
checked with synthetic engine tests; an in-session GPU map transition was not
exercised. `graphify update .` completed with parser/version/community-label
warnings. Formatting and diff checks passed. Logs and screenshots are under
`.superpowers/sdd/2026-10-07-programmable-live-map/`.
