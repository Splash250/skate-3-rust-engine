# Downtown Apartment Entry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Install a walkable Downtown apartment with matching live-map pins, configurable GTA V style entry/exit markers, controller prompts and multiplayer-approved travel.

**Architecture:** A native location service consumes immutable, bounded interior catalogs and mutable owner-scoped marker settings. Client and authority prepare the same collision transforms before travel; existing resource state/events, teleport approvals and admission identities carry multiplayer configuration. Render and minimap geometry have a distinct interior context, preserving the exterior map and its streaming lifetime.

**Tech Stack:** Rust, Bevy, existing recovered native collision/travel, serde JSON, existing Lua/resource runtime, Python NumPy/Pillow asset tooling. No new external dependency or network packet format.

**Spec:** `docs/superpowers/specs/2026-10-08-downtown-apartment-entry-design.md`

## Global Constraints

- Work on existing `proper-map`; preserve unrelated edits; no commit or push.
- Native defaults work in singleplayer; dedicated entry requires host approval and matching interior collision. No local teleport bypass during native-input verification.
- Preserve source texture colors and baked markings; no hologram treatment.
- One installed destination: Modern Apartment. Support multiple destinations without fabricating floors.
- A / PlayStation cross / E interact only while the player's physical volume intersects the marker.
- Ready travel begins immediately; readiness timeout is ten seconds, not a mandatory countdown.
- Keep large/private model assets in ignored local installation storage; keep creator/source attribution; do not recreate the deleted permission note.
- Downloaded resource budgets remain 16 MiB model, 2048px image sides, 16 MiB decoded texture references, 16 MiB expanded geometry; no glTF extensions.
- Retire generation-scoped UI/markers/assets safely; restore occupants before removing supporting collision.

## Review Focus

- Retail RWCM collision and authored collision must both retain their original queries after adding interior triangles (Task 3).
- Controller reconnect or menu focus changes while A is held must not activate entry or a skating action (Task 5).
- Resource retirement during loading/occupancy must reject stale completions and preserve safe support until return (Tasks 4 and 7).
- A client with different collision bytes or a forged floor request must not enter the authoritative interior (Task 7).
- Interior map zoom, exit and streamed Downtown reload must preserve exterior bounds and restore input ownership (Task 6).

## File structure and shared interfaces

New paths below are proposed implementation files; existing paths were inspected.

- `crates/skate-resources/src/locations.rs`: shared catalog/style/request schemas and validation, no ECS or rendering.
- `crates/skate-data/src/location_collision.rs`: bounded collision-shell decoding and transformed triangles; no ECS.
- `crates/skate-game/src/locations.rs` with `locations/{catalog,runtime,collision,markers,input,hud,map}.rs`: native integration, separate units by responsibility.
- `crates/skate-mods/src/{locations,resource_locations}.rs`: Lua command contract and owner state publication.
- `crates/skate-game/src/modding/locations.rs`: mod/resource adapter to the native service.
- `crates/skate-server/src/locations.rs`: host catalog readiness, entry validation and teleport coordination.
- `tools/prepare_interior.py`, `tools/test_prepare_interior.py`: reproducible GLB/collision preparation using explicit input/output paths.
- `content/locations/downtown/catalog.json`: lightweight shipped configuration; model and collision payloads installed locally.
- `sdk/LOCATIONS.md`, `sdk/examples/apartment-locations/`: contract and synthetic runnable example.

Shared schema types live in skate-resources, already consumed by game/mod/server.
`Catalog {version:1, map:String, interiors:Vec<Interior>, locations:Vec<Location>}`;
`Interior {key, model, collision, transform, spawn, heading, exit}`;
`Location {key,label,position,style,floors:Vec<Floor>}`;
`Floor {key,label,interior}`; `MarkerStyle {color:[f32;3],opacity,radius,height}`.
Paths are package-contained. Catalog geometry/transforms/spawns are immutable for
an admitted generation; mutable snapshots change styles, labels, enabled flags
and script interactions, never silently replace authoritative collision.

`LocationSnapshot`, `EntryIntent {location,floor,generation,request}` and
`LocationStatus` share canonical string generations. Default entry style is
RGB `[1.0,0.8,0.15]`, opacity `0.35`, radius `1.0m`, height `2.0m`.
Limits: 32 locations, 16 interiors, 8 floors/location, 64-byte keys/labels,
8 KiB mutable snapshot, five publications/second. Explicit grants are
`engine.locations` and `resource.locations`; teleport still requires
`resource.teleport`. These are new contracts, not existing APIs.

---

### Task 1: Catalog and collision-shell contracts

**Files:** Create shared schema files above; modify `crates/skate-resources/src/{lib,manifest,content}.rs`, `crates/skate-data/src/lib.rs`; tests inline in new modules and existing manifest tests.

**Interfaces:** Produce `Catalog::parse(bytes:&[u8])->Result<Catalog,String>`, `LocationSnapshot::parse(Value)->Result<LocationSnapshot,String>`, and `location_collision::decode(bytes:&[u8], transform:[[f32;4];4])->Result<Vec<[[f32;3];3]>,String>`. Collision shell is version-1 JSON with explicit indexed positions/triangles, capped at 65536 vertices, 32768 triangles and 4 MiB encoded bytes.

- [ ] Write failing tests `location_catalog_rejects_escape_nan_duplicate_and_missing_floor`, `location_snapshot_bounds_styles_and_counts`, and `collision_shell_rejects_indices_and_nonfinite_transforms`; assert rejected inputs leave prior snapshots unchanged.
- [ ] Run `cargo test --locked -p skate-resources location` and `cargo test --locked -p skate-data location_collision`; confirm actual new tests fail before implementation.
- [ ] Implement types/validators and optional manifest `locations` catalog path. Require its model/collision files in the manifest's existing file list; include them in normal content hashing, not state payloads.
- [ ] Repeat focused tests; assert valid synthetic floor/wall transforms produce exact expected world coordinates and malformed/path/size inputs fail.

### Task 2: Reproducible apartment asset installation

**Files:** Create `tools/prepare_interior.py`, `tools/test_prepare_interior.py`, `content/locations/downtown/catalog.json`; inspect `tools/asset_pipeline/versions.py` before any installer hook. Keep output under ignored local asset storage.

**Interfaces:** Produce `prepare(source:Path, output:Path)->dict` plus CLI `python3 tools/prepare_interior.py --source SOURCE.glb --output DIRECTORY`. Output `apartment.glb`, `collision.json`, hashes and source attribution. Accept a separately authored simplified shell; never assume visual triangles equal collision.

- [ ] Add synthetic tests `embedded_images_fit_default_resource_budget`, `baked_material_has_supported_equivalent`, `source_is_unchanged`, `shell_floor_and_doorway_match_visual_transform`; verify original bytes remain identical.
- [ ] Run `python3 -m unittest tools.test_prepare_interior -v` and confirm failure.
- [ ] Implement bounded texture resizing/packing and extension-free baked-color equivalent. Budget all images by decoded references, not JPEG bytes. Retain original source separately; normalize Y-up metre origin once. Author shell for floor, walls and major furniture, with an open internal path to the exit.
- [ ] Run tests, Khronos GLB validation, and the game's existing resource GLB validator against the runtime output. Reject output that does not fit policy. Compare baked colors close-up during Task 10.

### Task 3: Native additive collision and world identity

**Files:** Create `crates/skate-game/src/locations/collision.rs`; modify `crates/skate-game/src/{skate_world,physics,config,map_transition}.rs`, `crates/skate-server/src/{lib,world,native_authority}.rs`; create `crates/skate-game/src/locations/collision_tests.rs`.

**Interfaces:** Produce `locations::collision::compose(base:Arc<BoardWorld>, shells:&[PreparedShell])->Result<Arc<BoardWorld>,String>` without mutating the base. Shared `catalog_revision` deterministically hashes sorted catalog/geometry/model content. With no catalog, existing map fingerprint is unchanged.

- [ ] Add failing tests `authored_and_rwcm_queries_survive_location_composition`, `shell_contact_and_wall_match_on_client_and_authority`, `missing_catalog_keeps_old_fingerprint`, `different_shell_changes_admission_identity`.
- [ ] Run `cargo test --locked -p skate-game --bin skate3rust locations::collision_tests`; run focused server location tests. Establish failures before code.
- [ ] Compose independent query pools with valid unique IDs/material metadata instead of routing retail archives through the authored-only loader. Keep stock terrain and streaming untouched. Rebuild composition only when catalog content changes; restore base when retired.
- [ ] Include catalog revision in existing admission/world identity. Ordinary dedicated hosting reads only bounded add-on shell metadata, retaining hash-only retail hosting. Resource native verifier workers receive the same admitted catalog files via their local scratch preparation/CLI and compose the same collision; do not change solver or packet schema.
- [ ] Run focused tests plus `cargo check --locked -p skate-game --bin skate3rust` and `cargo check --locked -p skate-server`; demonstrate no-add-on hosting remains compatible.

### Task 4: Native loading and travel state machine

**Files:** Create `crates/skate-game/src/locations.rs`, `locations/{catalog,runtime}.rs`; modify `crates/skate-game/src/{main,app,map_transition}.rs`; tests inline.

**Interfaces:** Produce `LocationPlugin`, owner-keyed `LocationRegistry`, `begin_entry(world:&mut World,intent:EntryIntent)->Result<(),String>`, `status(world:&World)->LocationStatus`, and `retire_owner(world:&mut World,owner:&str,generation:u64)`. Runtime phases: Exterior, Preparing, AwaitingApproval, Interior, Returning. Request/generation tags accompany all completions.

- [ ] Write failing tests `travel_waits_for_scene_collision_and_support`, `ten_second_timeout_keeps_exterior_pose`, `retired_generation_discards_load_completion`, `occupied_retirement_returns_before_collision_release`.
- [ ] Run `cargo test --locked -p skate-game --bin skate3rust locations::runtime`; confirm failures.
- [ ] Prepare model, shell and safe spawn off the frame's critical path; publish atomically. Native travel uses existing SkaterRuntime travel/reset. Return spawn is grounded and outside the trigger. Keep support until successful return; failure cannot strand an actor.
- [ ] Repeat tests and confirm timeout exactly at ten seconds using synthetic time, no sleep/GPU dependency. Wire lifecycle to startup and committed CurrentMap replacement.

### Task 5: World markers, prompts and floor selection

**Files:** Create `locations/{markers,input,hud}.rs`; modify `crates/skate-game/src/input.rs`, existing `modding/interactions.rs` only at arbitration hooks. Reuse existing controller `prompt_style`/face labels.

**Interfaces:** Produce `intersect_marker(player_volumes:&[CollisionVolume], marker:&Location)->bool`, nearest eligible selection, `LocationInput` edge state, and native prompt/floor menu. Custom script actions emit owner-scoped `location_interact` events through the existing callback path.

- [ ] Add failing tests `intersection_uses_physical_volume_not_root_point`, `nearest_overlap_is_stable`, `held_a_focus_and_reconnect_require_new_edge`, `interaction_consumes_skating_action`, `single_floor_direct_multi_floor_select_cancel`.
- [ ] Run `cargo test --locked -p skate-game --bin skate3rust locations`; verify the new tests fail.
- [ ] Render a translucent RGB cylinder and ground ring with normal world depth testing; ignore them in gameplay collision. Add a camera-facing vector house icon. Top-left copy: `Press A to enter`, PlayStation cross equivalent, or `Press E to enter`; inside: `... to exit`. Multiple floors open an accessible named list, with cancel and focus release.
- [ ] Run tests; verify input ordering against Input → Controls → Physics and existing menu/map/browser arbitration. A selected interface does not make a held A activate skating on close.

### Task 6: Apartment map context and native pins

**Files:** Create `locations/map.rs`; modify `crates/skate-game/src/map_view/{view,geometry,markers,overlay,input}.rs` at context hooks, preserving existing public Lua map API.

**Interfaces:** Produce `MapContext::{Exterior,Interior {owner,key,generation}}` and `set_context(world:&mut World,context:MapContext)`. Use prepared full interior mesh/material data for its LODs and context-local bounds.

- [ ] Add failing tests `interior_pocket_does_not_expand_downtown_bounds`, `map_context_preserves_free_view_and_restores_exterior`, `pins_and_you_use_same_transform`, `streaming_does_not_drop_interior_overview`.
- [ ] Run `cargo test --locked -p skate-game --bin skate3rust map_view` and the new location map tests; confirm new assertions fail first.
- [ ] Add house pin/Apartment label outside and Exit pin inside. Switch context only after travel commits; preserve YOU/peer names and existing free-pan/top-down/tilt controls. Filter location pins/peers by visible context without changing transport identity.
- [ ] Repeat tests; restore exterior bounds and navigation when exiting or retiring. Scope compact/expanded rendering to the selected context, not the union of distant pockets.

### Task 7: Dedicated entry approval and content readiness

**Files:** Create `crates/skate-server/src/locations.rs`; modify `crates/skate-server/src/{lib,resources,world,native_authority}.rs`, `crates/skate-game/src/modding/resources.rs`; add `crates/skate-server/tests/locations.rs`.

**Interfaces:** Host `Locations::request(actor:u64,intent:EntryIntent,server:&mut Server)->Result<(),String>` resolves its own catalog destination. Use existing resource event envelopes and reserved state `__locations_v1` with generations/request IDs. Client readiness references admitted catalog revision; it never supplies coordinates.

- [ ] Write failing integration tests `forged_floor_and_far_entry_are_rejected`, `content_revision_mismatch_cannot_enter`, `one_actor_exit_does_not_move_peer`, `owner_stop_returns_occupants`, `native_verifier_uses_same_shell`.
- [ ] Run `cargo test --locked -p skate-server --test locations`; establish failures.
- [ ] Validate admitted actor, world/generation, physical reach and known floor, then use existing server-approved travel/leases. For catalog-installed native defaults, register an engine-owned provider through existing resource runtime/event delivery automatically; no manual server script installation is required. Preserve explicitly configured resources/grants. Native marker settings are presentation, not authority.
- [ ] Run integration tests and existing teleport-lease tests. Keep public shared interior occupancy visible to peers; do not disable native verification to accept interior movement.

### Task 8: Programmable Lua ownership and declarations

**Files:** Create `crates/skate-mods/src/{locations,resource_locations}.rs`, `crates/skate-game/src/modding/locations.rs`; modify mods `lib.rs`, `vm.rs`, `api.lua`, `resources.rs`, game `modding/{mod,resources}.rs`, `sdk/{skate,resource}.lua`; create `sdk/LOCATIONS.md` and a synthetic `sdk/examples/apartment-locations/`.

**Interfaces:** `sdk.locations.set(snapshot)`, `.clear()`, `.status()`; `resource.locations.set(snapshot)`, `.get()`, `.clear()`. Snapshot references prepared catalog keys, never replaces collision transforms. Server publication requires `resource.locations`; local client mutation requires `engine.locations`. Script markers can select generic cylinder/ring/icon and dispatch interaction events.

- [ ] Add failing tests for ownership, side/grant checks, count/byte/rate bounds, invalid replacement retention, stale state and retirement; include resource runtime tests.
- [ ] Run `cargo test --locked -p skate-mods locations` and focused `resource_runtime` tests; confirm failure.
- [ ] Implement wrappers/schema/host/state lifecycle together; reserve the state key against direct `resource.state.set`. Update discovery, declarations, grants and docs with the exact limits above. Example uses synthetic meshes/coordinates, not the downloaded asset.
- [ ] Repeat tests and `cargo run --locked -p skate-mods --example check_mod -- sdk/examples/apartment-locations`; confirm callbacks separately in Task 10.

### Task 9: Install Downtown catalog and final placement

**Files:** Complete `content/locations/downtown/catalog.json`; local ignored installation payloads; update `docs/LINUX.md` with actual install/default discovery paths.

**Interfaces:** Auto-discover map-relative `locations/<map-stem>/catalog.json` on client and host, with optional explicit `--locations DIRECTORY` for test fixtures. Resource manifest catalogs resolve only within their admitted package. Missing optional stock catalog disables that location without breaking the map.

- [ ] Add failing tests `optional_catalog_absence_keeps_stock_map`, `catalog_discovery_is_contained_and_deterministic`, `unsafe_entry_or_return_is_rejected`.
- [ ] Run focused location catalog tests.
- [ ] Use a live Downtown doorway and native collision/support queries to record entrance/return coordinates. Reserve apartment pocket around `[4096,100,4096]`, adjusting only if world queries reject it; record final values in catalog. Pick the interior spawn/exit from actual floor geometry and verify clear standing room. Do not fabricate doorway coordinates from screenshots.
- [ ] Run discovery tests, read back installed hashes, and verify both client and host discover the identical catalog. Preserve attribution; no permission note and no large asset staged in Git.

### Task 10: Live acceptance, review and completion evidence

**Files:** Scratch logs/captures under `.superpowers/sdd/2026-10-08-downtown-apartment-entry/`; update scoped docs only with observed behavior.

- [ ] Run affected package checks and focused tests from preceding tasks; confirm nonzero test counts. Scope formatting to changed Rust files. Record prerequisite and pre-existing failures.
- [ ] Build the game once using existing target/default dev-dynamic configuration; avoid parallel builds sharing target.
- [ ] Run owned-asset Downtown singleplayer: marker/map alignment, E entry, floor/wall/furniture walking, camera, exit, source/runtime material comparison, map pan/zoom/2D/3D, timeout/error path. Capture exterior and interior screenshots.
- [ ] Run two dedicated clients: shared entry, peer names in apartment, independent exit, reconnect, retirement and map change. Exercise controller labels synthetically and physical controller buttons if hardware is available; distinguish these checks.
- [ ] Exercise verified native-input mode in an admitted resource world containing the same catalog; verify solver collision and digest match. Do not claim this from an ordinary two-client check.
- [ ] Obtain a whole-change review using the execution method selected by the user; fix material issues and rerun affected checks only.
- [ ] Run `graphify update .`; inspect final diff/status for unrelated changes and private assets. Report actual checks and link captures. No commit/push.

## Handoff and self-review

The plan covers all approved spec sections: source/runtime asset budgets (2),
collision/world identity (3), readiness/timeout/safe retirement (4), GTA markers
and floors (5), interior map (6), multiplayer authority (7), programmable owners
(8), actual placement/default installation (9), live evidence (10). Task 1 pins
the shared types used by later tasks; each Review Focus condition has a named
owning regression test. Execution must read both the spec and this plan.

No code, assets or dependency installation should start until the user has
reviewed this plan and selected an execution method. Existing work remains in
the current branch; any isolation must preserve its dirty feature state.
