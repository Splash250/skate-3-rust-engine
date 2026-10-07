# Boardwalk Borough roleplay showcase Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Each task owns its files and verification. Do not commit unless the user authorizes commits.

**Goal:** Deliver a playable, account-backed roleplay showcase with a custom skate-town, phone apps, proximity/radio voice, pizza work, and private apartments.

**Architecture:** Reuse the existing phone, profile, resource, database, voice, teleport, instance, and park-authoring APIs. Add server resources for a shared idempotent economy, property leases, and pizza jobs; let those resources register their own phone apps. Distribute the original world and exact resource files through one digest-pinned server pack.

**Tech Stack:** Rust/Bevy host APIs; Lua server/client resources; HTML/CSS/JavaScript resource pages; SQLite service; Python park and server-pack tooling; Cargo integration tests; Linux/Windows CI.

**Spec:** `docs/superpowers/specs/2026-10-05-boardwalk-borough-rp-showcase-design.md`

## Global Constraints

- Keep the showcase engine-free; use existing APIs unless a concrete missing capability blocks a requirement.
- Key durable player records by `platform-profiles.identity(actor)`, never by transient connection actor ID.
- Keep job phase, prices, rewards, property access, marker IDs, and teleporter destinations server-owned.
- Treat database work as asynchronous. Give every wallet operation a stable ID and make retries return its original result.
- Resolve cross-resource purchase/reward crash windows by persisting the caller's pending operation ID and retrying the same idempotency key after restart.
- Keep resource databases separate; do not open another resource's database files directly.
- Keep phone HTML as presentation and action input only; authenticated server handlers decide state changes.
- Keep voice opt-in, physical V/LB push-to-talk, local mute/deafen, and instance isolation intact.
- List every shipped resource file in its manifest and pin every manifest-declared file in the server-pack recipe with its digest, byte count, and provenance.
- Use only original or redistributable map/UI assets. Keep retail assets, credentials, private settings, and databases out of downloadable content.
- Preserve unrelated worktree changes. The approved spec is currently uncommitted; do not overwrite or stage it.
- Run `graphify update .` after code changes and report any failure.
- Do not commit until the user asks.

## Review Focus

- Replayed, mismatched, or late wallet operation IDs must not double-charge or double-pay; pin this in Task 2's SQLite recovery tests.
- A purchase charged immediately before a property-resource restart must reconcile to one owned lease; pin this in Task 4's restart test.
- Forged sender/account IDs, stale actors, wrong-instance requests, and out-of-order stops must not mutate another player's state; pin these in Tasks 2, 4, and 5.
- A failed, timed-out, or retired teleport must not strand a player or admit an uninvited guest; pin this in Task 4 and in the actual-server readmission check in Task 6.
- Browser/voice startup failure and missing native devices must leave the player able to skate and must not grant fake job, property, or call success; pin the UI lifecycle test in Task 6 and the manual device check in Task 7.

---

## File Map

- `resources/boardwalk-borough/` contains the generated park scene, marker data, world manifest, and map-specific server/client guidance.
- `resources/rp-economy/` owns account-keyed wallet/ledger data and idempotent charge/credit operations.
- `resources/rp-properties/` owns apartment policy, pending lease operations, private-room access, and its phone page.
- `resources/rp-pizza/` owns shift/order progression, delivery checks, dispatch membership, and its phone page.
- `resources/voice-room/` gains the showcase proximity setting and a narrow server export for job dispatch membership; its native client push-to-talk bridge remains in place.
- `resources/packs/boardwalk-borough.recipe.json` pins the complete runnable set. `docs/multiplayer/boardwalk-borough.md` documents installation and the manual acceptance run.
- `tools/test_boardwalk_borough.py`, `crates/skate-server/tests/gameplay_resources.rs`, `crates/skate-mods/tests/phone_ui.rs`, and `.github/workflows/networking.yml` cover content, resource behavior, UI registration, and cross-platform test execution.

### Task 1: Author the town world and marker contract

**Files:**
- Create: `resources/boardwalk-borough/placements.json`
- Create: `resources/boardwalk-borough/placements-low.json`
- Create: `resources/boardwalk-borough/park.skate`
- Create: `resources/boardwalk-borough/park-low.skate`
- Create: `resources/boardwalk-borough/markers.json`
- Create: `resources/boardwalk-borough/resource.json`
- Create: `resources/boardwalk-borough/server.js`
- Create: `resources/boardwalk-borough/client.lua`
- Create: `resources/boardwalk-borough/README.md`
- Create: `tools/test_boardwalk_borough.py`
- Modify: `crates/skate-server/tests/gameplay_resources.rs`

**Interfaces:**
- Consumes: `tools/resource_park.py` format-1 placements and generated marker records; `resource.players()`, scoped state, `sdk.readText`, and approved `resource.teleport` APIs.
- Produces: one required-world resource ID `boardwalk-borough`; stable markers `plaza_spawn`, `pizza_counter`, `drop_01`, `drop_02`, `drop_03`, `apt_entry`, and `apt_exit`; player-scoped marker guidance consumed by job/property resources.

- [x] **Step 1: Add failing content-contract tests**

Add `test_boardwalk_marker_manifest_matches_placements` and
`test_boardwalk_lod_and_base_export_are_valid` to
`tools/test_boardwalk_borough.py`. Assert portable unique object/marker IDs,
presence and type of every required marker, matching generated positions and
labels, a collidable base scene, and a render-only low LOD.

- [x] **Step 2: Run the focused tests and confirm they fail**

Run: `python3 -m unittest tools.test_boardwalk_borough -v`
Expected: FAIL because the Boardwalk scene and manifest do not exist yet.

- [x] **Step 3: Create and export the original town scene**

Use `tools/resource_park.py` or Park Studio to author a plaza, pizza storefront,
apartment exterior, skate obstacles, spawn points, delivery locations, and a
simple interior area at a separate world position. Place marker objects in
`placements.json`; generate `markers.json` and both `.skate` files through the
existing exporter. Use original geometric primitives and palette choices.

- [x] **Step 4: Add the required-world resource behavior**

Create `resource.json` with explicit world/map, LOD, script, marker, and
placement file declarations. In `server.js`, load `markers.json` with
`sdk.readText`, allocate stable public spawn pads, detect marker entry from
`resource.players()` observations, and publish bounded player-scoped guidance.
In `client.lua`, render and clean up that guidance with `sdk.ui.text`.

- [x] **Step 5: Add the runtime regression and rerun map tests**

Add `boardwalk_map_markers_are_server_observed_and_instance_scoped` to
`crates/skate-server/tests/gameplay_resources.rs`. Install the map resource in
the real resource Host fixture; assert entry guidance comes from the expected
server-observed marker, does not advance in a different instance, and disappears
when the actor leaves. Then run:

```sh
python3 -m unittest tools.test_boardwalk_borough tools.test_resource_park tools.test_park_editor -v
cargo test --locked -p skate-server --test gameplay_resources boardwalk_map_markers_are_server_observed_and_instance_scoped
```

Expected: all three Python suites and the named Rust test pass.

### Task 2: Implement the shared persistent economy

**Files:**
- Create: `resources/rp-economy/resource.json`
- Create: `resources/rp-economy/server.lua`
- Create: `resources/rp-economy/README.md`
- Modify: `crates/skate-server/tests/gameplay_resources.rs`

**Interfaces:**
- Consumes: exact dependency `platform-profiles:1.0.0`; database service transactions/migrations; admitted `resource.players()` snapshots.
- Produces: server exports `balance({actor})`, `submit({actor,operation_id,kind,amount,reason})`, and `operation({actor,operation_id})`. `submit` accepts only `kind="charge"` or `kind="credit"` and returns an accepted/pending result; `operation` returns `pending`, `applied`, or `rejected` plus the resulting balance/error. The client receives only its own balance and operation results through player-scoped state/events.

- [x] **Step 1: Add failing wallet and replay tests**

Add `wallet_debits_are_conditional_and_idempotent`,
`wallet_credits_are_idempotent_and_reconnect_persists`, and
`economy_rejects_unverified_and_cross_actor_operations` to
`crates/skate-server/tests/gameplay_resources.rs`. Use the existing real Lua
Host plus SQLite `Services` fixture. Assert insufficient funds roll back,
repeating the same ID/content returns the original receipt, reusing an ID with
different content is rejected, and a new actor for the same account sees the
same balance.

- [x] **Step 2: Run the wallet tests and confirm they fail**

Run: `cargo test --locked -p skate-server --test gameplay_resources wallet_`
Expected: compile/test failure because `rp-economy` and its exports are absent.

- [x] **Step 3: Define bounded wallet settings and schemas**

Declare the `platform-profiles` dependency, `resource.exports`,
`resource.database`, `resource.network`, `resource.state`, and
`resource.settings` capabilities. Add typed settings for starter balance and
maximum balance/operation amount. Migrate account-keyed wallet and operation
receipt tables with SQL checks, a unique operation ID, canonical action/amount
fields, and integer credit units.

- [x] **Step 4: Implement idempotent asynchronous operations**

Resolve `actor` with `resource.call("platform-profiles", "identity", actor)`
and confirm it is still admitted. Submit bounded SQL transactions; do not
pretend the export call itself proves a commit. Cache pending/results, persist
the operation receipt in the same transaction as the balance change, and make
`operation` reconcile unknown IDs from the receipt table after a resource/server
restart. Publish a private balance snapshot only to the current actor mapped to
that account.

- [x] **Step 5: Run wallet recovery and existing account regressions**

Run:

```sh
cargo test --locked -p skate-server --test gameplay_resources wallet_
cargo test --locked -p skate-server --test gameplay_resources economy_rejects_unverified_and_cross_actor_operations
cargo test --locked -p skate-server --test accounts
```

Expected: wallet tests pass and existing account behavior remains green.

### Task 3: Configure proximity voice and job dispatch membership

**Files:**
- Modify: `resources/voice-room/resource.json`
- Modify: `resources/voice-room/server.lua`
- Modify: `resources/voice-room/README.md`
- Modify: `crates/skate-server/tests/gameplay_resources.rs`

**Interfaces:**
- Consumes: existing host voice operation API and selected channel client event.
- Produces: typed private `proximity_meters` setting (default 12, range 1–100); declared export `dispatch_member({actor,enabled}) -> {ok,error?}`; reserved channel `voice-room/pizza_dispatch`, whose membership is set only by the server resource. The client retains the existing `--voice` and V/LB gates.

- [x] **Step 1: Add failing voice policy and membership tests**

Add `voice_policy_uses_operator_radius_and_dispatch_membership_is_authoritative`
to `crates/skate-server/tests/gameplay_resources.rs`. Assert startup submits
the configured proximity operation; only admitted actors can join; enabling
adds the actor and sends channel selection; disabling, disconnect, and resource
retirement remove membership and reset client selection.

- [x] **Step 2: Run the named test and confirm it fails**

Run: `cargo test --locked -p skate-server --test gameplay_resources voice_policy_uses_operator_radius_and_dispatch_membership_is_authoritative`
Expected: FAIL because voice-room currently has neither a proximity setting
nor a dispatch export.

- [x] **Step 3: Add the setting and narrow dispatch export**

Declare `resource.settings.v1`, `resource.exports`, and bounded
`proximity_meters`. Submit proximity at load and on live setting changes.
Register `dispatch_member`, verify the actor is admitted, update only the
reserved dispatch channel, and send `select_channel` only to affected players.
Clear memberships on disconnect and resource unload. Keep administrator radio
commands and the existing client prefix validation intact.

- [x] **Step 4: Run voice policy and server voice regressions**

Run:

```sh
cargo test --locked -p skate-server --test gameplay_resources voice_policy_uses_operator_radius_and_dispatch_membership_is_authoritative
cargo test --locked -p skate-server --test voice
```

Expected: both pass; no resource can use this export to select an unadmitted
actor or bypass the native voice gate.

### Task 4: Add persistent apartments and private interior trips

**Files:**
- Create: `resources/rp-properties/resource.json`
- Create: `resources/rp-properties/server.lua`
- Create: `resources/rp-properties/client.lua`
- Create: `resources/rp-properties/index.html`
- Create: `resources/rp-properties/properties.css`
- Create: `resources/rp-properties/properties.js`
- Create: `resources/rp-properties/README.md`
- Modify: `crates/skate-server/tests/gameplay_resources.rs`

**Interfaces:**
- Consumes: exact dependencies `platform-profiles:1.0.0`, `rp-economy:1.0.0`, `boardwalk-borough:1.0.0`, and `phone:1.0.0`; map marker IDs; approved teleport leases and bounded instances; phone app registry.
- Produces: `request` network actions `list`, `rent`, `enter`, `exit`, `invite`, and `accept_invite`; player-scoped property snapshots/results; phone app registered by `sdk.ui.interfaces.register("open", {version=1, label="Properties", destination="phone", phone=true, ...})`.

- [x] **Step 1: Add failing lease, authorization, and recovery tests**

Add `property_purchase_recovers_a_charged_operation_after_restart` and
`property_entry_requires_owner_or_invite_and_returns_to_previous_world` to
`crates/skate-server/tests/gameplay_resources.rs`. Assert duplicate rent charges
once, a lost property-side acknowledgement reconciles against the same economy
operation ID, an uninvited account cannot enter, a guest shares only the owner's
private instance, and exit/retirement restores the saved position safely.

- [x] **Step 2: Run the named tests and confirm they fail**

Run: `cargo test --locked -p skate-server --test gameplay_resources property_`
Expected: FAIL because `rp-properties` is absent.

- [x] **Step 3: Implement account leases and recovery state**

Create a small fixed unit catalog, one active lease per account, a pending
purchase/lease operation record, and bounded invitation records. Rent requests
persist a stable operation ID before asking `rp-economy` to charge; poll/retry
that same ID after restart; activate the lease only after an applied receipt.
Guests receive access through account-bound invitations resolved to live actors.

- [x] **Step 4: Implement isolated entry/exit and the phone page**

Allocate bounded private instance IDs from a configured pool; use only the
unit's predefined interior spawn and `restore_on_stop` teleport leases. Reject
client coordinates and arbitrary instance IDs. Add the property app, render the
server snapshot, and send only named requests. Clean up browser surfaces and
pending UI correlations on close, disconnect, or resource retirement.

- [x] **Step 5: Run property, phone lifecycle, and teleport regressions**

Run:

```sh
cargo test --locked -p skate-server --test gameplay_resources property_
cargo test --locked -p skate-server --test gameplay_resources resource_stop_and_restart_restore_temporary_travel_through_real_udp_readmission
cargo test --locked -p skate-mods --test phone_ui
```

Expected: all named tests pass; existing phone open/close and camera focus
behavior remains unchanged.

### Task 5: Add the pizza shift and dispatch flow

**Files:**
- Create: `resources/rp-pizza/resource.json`
- Create: `resources/rp-pizza/server.lua`
- Create: `resources/rp-pizza/client.lua`
- Create: `resources/rp-pizza/index.html`
- Create: `resources/rp-pizza/pizza.css`
- Create: `resources/rp-pizza/pizza.js`
- Create: `resources/rp-pizza/README.md`
- Modify: `crates/skate-server/tests/gameplay_resources.rs`

**Interfaces:**
- Consumes: exact dependencies `platform-profiles:1.0.0`, `rp-economy:1.0.0`, `voice-room:1.0.0`, `boardwalk-borough:1.0.0`, and `phone:1.0.0`; generated marker IDs; current admitted player observations.
- Produces: `request` actions `start`, `pickup`, `deliver`, `cancel`, and `snapshot`; server-owned route/status; phone app registered under `rp-pizza/open`; economy credit with a server-generated idempotency ID; dispatch membership toggled through the `voice-room.dispatch_member` export.

- [x] **Step 1: Add failing route, payout, and dispatch tests**

Add `pizza_route_rejects_forged_stale_out_of_order_and_wrong_instance_actions`
and `pizza_payout_and_dispatch_recover_exactly_once_after_restart` to
`crates/skate-server/tests/gameplay_resources.rs`. Assert the route can advance
only at the configured marker sequence and minimum server elapsed time; a
payload cannot choose a target, reward, sender, or account; a completed payout
is credited once through a lost acknowledgement/restart; dispatch is removed
on finish, cancellation, disconnect, timeout, and retirement.

- [x] **Step 2: Run the named tests and confirm they fail**

Run: `cargo test --locked -p skate-server --test gameplay_resources pizza_`
Expected: FAIL because `rp-pizza` is absent.

- [x] **Step 3: Implement the server-owned shift state machine**

Support one active shift per account with states `offered`, `active`,
`picked_up`, `payout_pending`, and `complete` (plus terminal cancel/expiry).
Load route IDs and positions from `boardwalk-borough/markers.json`; never accept
a client-selected route or amount. Validate actual sender, expected marker,
current instance, cooldown/elapsed-time floor, state order, and generation.
Persist the pending payout ID before submitting a wallet credit; replay it after
restart and mark completion only after `rp-economy.operation` reports applied.

- [x] **Step 4: Connect dispatch and add the phone UI**

On shift start, request `voice-room.dispatch_member({actor,enabled=true})`; clear
it on every terminal path. Register a phone Jobs app and render the current
pickup/dropoff and result from server state. Submit only named actions and show
pending/error states until authoritative results arrive.

- [x] **Step 5: Run the pizza, economy, UI, and existing call regressions**

Run:

```sh
cargo test --locked -p skate-server --test gameplay_resources pizza_
cargo test --locked -p skate-server --test gameplay_resources wallet_
cargo test --locked -p skate-mods --test phone_ui
cargo test --locked -p skate-server --test phone_calls
```

Expected: all tests pass, including actual TLS login and encrypted loopback
voice-routing coverage in the existing call test.

### Task 6: Assemble and document the pinned showcase pack

**Files:**
- Create: `resources/packs/boardwalk-borough.recipe.json`
- Create: `docs/multiplayer/boardwalk-borough.md`
- Create: `tools/test_boardwalk_borough_pack.py`
- Modify: `tools/test_server_pack.py`
- Modify: `.github/workflows/networking.yml`
- Modify: `crates/skate-server/tests/gameplay_resources.rs`
- Modify: `crates/skate-mods/tests/phone_ui.rs`

**Interfaces:**
- Consumes: all resource manifests from Tasks 1–5, current account bootstrap and browser-host requirements, `tools/server_pack.py` recipe schema, and the existing Linux/Windows networking workflow.
- Produces: account-required `boardwalk-borough` recipe with exact file pins, complete grants/settings/ensure closure, a documented install/start sequence, and regression tests that exercise the resource list and app registration.

- [x] **Step 1: Add failing pack-closure and app-registration tests**

Add `boardwalk_recipe_pins_every_manifest_file_and_uses_one_required_world`
to `tools/test_boardwalk_borough_pack.py`. Assert all declared resource files
are pinned, digests/byte counts match local sources, dependencies are present,
the recipe requires accounts, and the only required world is
`boardwalk-borough`. Add `boardwalk_phone_apps_register_and_retire_cleanly`
to `crates/skate-mods/tests/phone_ui.rs`; assert Jobs and Properties appear as
phone interfaces and their browser/input ownership cleans up on stop.

- [x] **Step 2: Run the focused tests and confirm they fail**

Run:

```sh
python3 -m unittest tools.test_boardwalk_borough_pack -v
cargo test --locked -p skate-mods --test phone_ui boardwalk_phone_apps_register_and_retire_cleanly
```

Expected: both fail because the recipe and apps do not exist.

- [x] **Step 3: Build the recipe from the complete resource dependency closure**

Pin `boardwalk-borough`, profiles, economy, properties, pizza, voice-room,
phone, phone-calls, interaction-policy, inventory-ui, admin-dashboard,
and their dependencies. Put every selected resource file in the
recipe with exact SHA-256, byte count, and provenance. Set only required grants,
12-metre proximity, fixed demo prices/rewards, and the single town world.
Compute final file pins only after content is stable.

- [x] **Step 4: Write the server runbook and pack/app tests**

Document account initialization, pack plan/apply/verify, browser companion
prerequisites for phone pages, resource startup, UDP/TCP firewall requirements,
operator commands, database backup location, and rollback limits. Add the pack
test and the UI lifecycle test without weakening existing tests.

- [x] **Step 5: Run pack validation and extend matrix CI**

Run:

```sh
python3 -m unittest tools.test_boardwalk_borough_pack tools.test_server_pack -v
python3 tools/server_pack.py plan resources/packs/boardwalk-borough.recipe.json --source-root resources
cargo test --locked -p skate-server --test package --test gameplay_resources
cargo test --locked -p skate-mods --test phone_ui
```

Update `.github/workflows/networking.yml` so the Linux/Windows matrix runs
`--test phone_ui` and the new pack/map Python tests. Verify the recipe with
`server_pack.py apply` into a disposable temp root and `verify`; then remove the
temp root after recording the result.

Expected: recipe validation accepts the pinned set on both supported CI OSes;
the manifest validator reports one required world and no missing grants/files.

### Task 7: Exercise the full local, cross-platform, and WAN scenario

**Files:**
- Modify: `docs/multiplayer/boardwalk-borough.md`
- Evidence: append an acceptance table and logs summary to the same document; do not commit user credentials, private account data, or packet captures containing private identifiers.

**Interfaces:**
- Consumes: verified pack from Task 6, trusted Linux/Windows game/server binaries, prepared client assets, account profiles, browser companion, working audio devices, and an externally reachable WAN endpoint.
- Produces: reproducible local instructions and recorded acceptance outcomes for the two-client/Linux, two-client/Windows, mixed-platform, and WAN tests.

- [x] **Step 1: Run the automated resource and server regressions**

Run:

```sh
cargo test --locked -p skate-server --test gameplay_resources
cargo test --locked -p skate-server --test phone_calls
cargo test --locked -p skate-mods --test phone_ui
python3 -m unittest tools.test_boardwalk_borough tools.test_boardwalk_borough_pack tools.test_resource_park tools.test_park_editor tools.test_server_pack -v
```

Expected: all targeted suites pass. Report unrelated pre-existing failures
separately; do not relax assertions to make the showcase green.

- [ ] **Step 2: Run the two-client Linux graphical scenario**

Start the pinned account-required server and two clients. Verify map admission,
phone launch/call, proximity voice inside and outside 12 metres, dispatch radio,
order pickup/delivery/payment, rent/payment, invite-only interior access, exit,
and balance/property persistence after both resource and server restart. Record
actual microphone/output devices and browser readiness.

- [ ] **Step 3: Repeat on Windows and mixed hosts/clients**

Use the Windows package and native browser/audio components. Repeat the same
scenario with a Windows host/client and a mixed Linux/Windows pairing. Record
unsupported prerequisites as pending; CI compilation is not graphical runtime
acceptance.

- [ ] **Step 4: Run the WAN scenario and record transport conditions**

Connect clients from separate networks to the authenticated account-enabled
server. Verify content transfer, admission, voice/call routing, instance
isolation, delivery, apartment access, and restart persistence. Record host and
client OSes, RTT/loss observations, forwarded UDP/TCP port, and voice-device
results. Do not describe loopback evidence as WAN evidence.

- [x] **Step 5: Refresh graph output and inspect the final diff**

Run `graphify update .`, `git diff --check`, and inspect `git status --short`.
Confirm only showcase code/content/docs and tests changed, all recipe pins match,
no secret/private/retail assets were added, and the cross-platform/WAN evidence
table accurately distinguishes passed, blocked, and unrun checks.
